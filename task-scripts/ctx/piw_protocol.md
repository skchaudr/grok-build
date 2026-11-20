# piw client protocol v1: adapter-server contract (read-only research)

All paths are under `the pi-workflows repo`. Cargo.toml and `~/.cargo/.crates.toml:7` both say the installed `piw` is pi-workflows 0.17.7. I did not run piw; everything below comes from reading source.

## 1. Transport, framing, handshake

**Socket type and path**
- Unix domain stream socket on Unix. `tui/src/client.rs:806-827` connects with `UnixStream::connect`. On Windows it is a named pipe.
- Default path is `~/.pi/agent/workflows/server/server.sock` (`tui/src/main.rs:56-70`).
- The env var `PIW_SOCKET` pins a different path (`main.rs:12,64-66,87-89`). An empty value means not pinned. There is no CLI flag for the socket.
- Path length limit is 103 bytes on macOS and 107 on Linux (`main.rs:105-126`).

**Autostart behaviour**
- Before connecting, piw runs `ensure_server` (`main.rs:137-164`). It does one `UnixStream::connect` probe and drops it at once. The adapter must tolerate a connection that closes with no data.
- If the probe fails and `PIW_NO_AUTOSTART` is unset, piw runs `pi-workflows server start` and waits up to 10 s (`main.rs:141-164`).
- `PIW_NO_AUTOSTART=1` turns that off (`main.rs:93-99`). Values "", "0", "false" mean off.
- The real connection is a second connect from `RemoteRuns::connect_local` (`client.rs:505,806`).

**Alternative transport**
- `piw --connect ws://host:port/path` (`main.rs:23,180-182`) goes to `ui::run_remote` and returns before `ensure_server` and the socket-length check.
- It uses `tokio_tungstenite::connect_async(url)` (`client.rs:771-804`). Each WebSocket text frame is one message with no newline. Binary frames are an error.
- `--connect` only works in TUI mode. It conflicts with `run_id` and `--once` (`main.rs:23,34`).
- The built-in relay is `piw serve`, which serves `/ws` (`server.rs:117-123`), but the client takes any ws URL.

**Framing**
- NDJSON: one JSON object per line, `\n` terminated (`client.rs:837-858`; the client writes `text + "\n"`).
- Max 1 MiB per message (`protocol.rs:205`).
- Every message in both directions must be canonical JSON. The client parses the text, re-serializes it, and requires byte equality, otherwise the connection drops (`protocol.rs:204-213`).
- Canonical rules (`protocol.rs:224-283`):
  - Keys sorted by UTF-16 code unit order, no whitespace.
  - Strings are escaped as serde_json does: `\"`, `\\`, `\b \t \n \f \r`, other C0 controls as lowercase `\u00xx`. Nothing else is escaped, including `/`, DEL and non-ASCII.
  - Integers are printed as ints. Non-integers use ECMAScript number formatting, so `1.0` is invalid and must be written `1`. `-0` becomes `0`. `1e+21` and `1e-7` follow the ECMAScript form.
- Any parse or validation error returns `Err` from `handle_server_message` (`client.rs:996-998`). That drops the connection, and piw reconnects with backoff 250 ms doubling up to 10 s (`client.rs:763`).

**Handshake**
- The client does not wait for hello. At the top of each loop iteration it sends `view.runs.watch` (and `view.run.watch` for a run given on the command line) before reading anything (`client.rs:841-847`). The adapter should write hello immediately on accept.
- Hello (`protocol.rs:84-94`, `deny_unknown_fields`): exactly `{"connectionId","packageVersion","schema":"pi-workflows.client.v1","type":"hello"}`.
- Validation (`protocol.rs:171-178`):
  - `schema` equals `pi-workflows.client.v1`.
  - `type` equals `hello`.
  - `connectionId` is 1 to 256 bytes.
  - `packageVersion` is non-empty.
- **Version gate** (`client.rs:998-1004`): `packageVersion` must equal piw's `CARGO_PKG_VERSION` exactly. For the installed binary that is `"0.17.7"`. A mismatch gives the error "workflow client version mismatch" and an endless reconnect loop.
- Until a valid hello arrives, `connected` stays false and the UI shows "connecting".

**Strictness by message type** (`protocol.rs:64-123`)
- Request: `deny_unknown_fields`.
- Hello, response, event: all `deny_unknown_fields`, with `#[serde(rename = "...")]` camelCase names.

## 2. Requests piw sends

Request envelope, which the adapter receives in canonical JSON:
- Fields: `schema`, `type:"request"`, `requestId`, `clientId`, `operation`, `idempotencyKey`, `payload`, and optional `runId` and `expectedRevision`.
- piw never sets `expectedRevision`.
- `requestId` equals `idempotencyKey`, formatted `piw-<pid>-<n>`, and `clientId` is `piw-<pid>` (`client.rs:975-994`).
- Commands use `piwc-<nonce>-<n>` for both ids (`client_commands.rs:45-60`).
- `runId` is omitted when none (`protocol.rs:77`).

Response envelope (`protocol.rs:96-108`):
- Fields: `schema`, `type:"response"`, `requestId`, `outcome`, optional `revision`, `receipt`, `error`.
- Outcomes: `accepted | adopted | rejected | conflict | notFound | claimLost | unavailable`.
- An empty-string `error` is invalid.

How responses are used (`client.rs:1028-1070`):
- If the `requestId` belongs to a tracked command, it completes that command.
- Otherwise, `unavailable`, `rejected` or `notFound` sets `shared.error` to the `error` text. This shows only in loading or empty panes and is cleared on the next hello.
- Otherwise, an `accepted` response with `receipt.schema` set to one of three values is merged. Everything else is ignored:
  - `pi-workflows.run-list-page.v1` is merged as a run list page. A non-accepted outcome clears the paging request.
  - `pi-workflows.run-page.v1` is merged as a page.
  - `pi-workflows.content-chunk.v1` is merged as a content chunk.
- Responses to watch and unwatch (TS sends `{"subscribed":true|false}`) need no particular shape, only valid envelope fields.

**Sent unprompted (minimum a server must answer)**

| Operation | When | runId | Payload | Expected reply |
|---|---|---|---|---|
| `view.runs.watch` | on every connect (`client.rs:884-892`) | absent | `{"subscriptionId":"runs"}` | Response `accepted`, any receipt. Then a `runs` event, now and on change. |
| `view.run.watch` | for the selected run, or the CLI run (`client.rs:924-934`) | the run id | `{"subscriptionId":"run:<runId>"}` | Response `accepted`, or `notFound` with an `error`. Then `run_snapshot` events. |

**Sent on demand (needed for correctness in some paths)**

| Operation | Trigger | Payload | Expected reply |
|---|---|---|---|
| `view.page` | scrubbing replay (`ui/mod.rs:725-745`) or the info page keys (`ui/mod.rs:978-1016`) | `{"kind":<kind>,"cursor":<u64>}` | `accepted` with a `run-page.v1` receipt (section 3). |
| `view.runs.page` | only if the `runs` event has `items.len() < total` (`client.rs:1098-1111,899-909`) | `{"revision":<string>,"cursor":<u64>}` | `accepted` with a `run-list-page.v1` receipt. |
| `view.content` | only if the snapshot contains `$artifact` refs (`client.rs:960-970`) | `{"path":<string>,"offset":<u64>}` | `content-chunk.v1` receipt. |
| `view.run.unwatch` | when the selection changes (`client.rs:910-923`) | `{"subscriptionId":"run:<id>"}` | `accepted`. |

Commands (optional, `client.rs:620-627,971`): `run.pause`, `run.resume`, `run.cancel`. The payload is `{}` and `runId` is set. They are retried after a reconnect with the same `idempotencyKey` and a new `requestId` (`client_commands.rs:62-66,114-127`). See section 6.

**Never sent by piw**
- Greps of `tui/src` show no other operation strings, so these are never sent: `view.run.get`, `view.session.watch`, `view.session.window`, `run.start`, `run.status`, `run.list`, `checkpoint.answer`, `decision.answer`, `interaction.*`, `server.status`, and the rest of `OPERATIONS` (`protocol.rs:8-44`).
- That list is only used by `parse_client_request`.
- Replying `rejected` to an unknown operation is safe.

## 3. Pages, snapshots, live updates

**Live updates are whole snapshots, not patches**
- The client handles only two events, `runs` and `run_snapshot`. Any other event is ignored (`client.rs:1011-1027`).
- `PatchOp` and `apply_patch` (`protocol.rs:298-331`) are used only in unit tests (`client.rs:1914-1930`). There is no patch event on the wire.
- The TS server never emits `run_page` either. `src/server/server.ts:1493-1537` emits only `runs`, `run_snapshot`, `session_snapshot` and `unavailable`.
- `docs/LIVE_REPLAY_PROTOCOL.md:101` says patches are "supported", but the Rust client does not consume them.

**Event envelope** (`protocol.rs:110-123`, `deny_unknown_fields`)
- Fields: `schema`, `type:"event"`, `subscriptionId`, `event`, `payload` (required), and optional `revision`, `runId`.
- `event` must be one of `runs, run_snapshot, run_page, session_snapshot, unavailable` (`protocol.rs:56-62`).
- Validation: `subscriptionId` is 1-256 bytes, and `runId` is 1-256 bytes if present.
- `run_snapshot` needs `runId` or it is dropped (`client.rs:1016-1018`). The client does not read `subscriptionId`.
- Every new `run_snapshot` replaces the stored view for the run, including any paged windows. The page requests are then re-sent automatically because the dedupe key includes the revision (`client.rs:935-954`).

**Revisions**
- A snapshot's revision is `payload.revision`, or the event `revision`, or 0 (`client.rs:1372-1382`).
- A page receipt is applied only if `receipt.revision` equals that value and `receipt.cursor` equals the currently requested cursor for `(runId, kind)` (`client.rs:1276-1292`).
- `graphRevision` (default: revision) is used only as a render cache key.
- The TS server bumps the per-subscription `revision` by 1 only when the payload digest changes (`server.ts:1509-1512`). It also polls every 250 ms. Neither is required of an adapter.

**Run snapshot payload** (`decode_view`, `client.rs:101-296`)

Required. A missing or invalid value gives Invalid, which shows "Workflow run snapshot is invalid: …":
- `manifest`: serde `Manifest` (`state/types.rs:196-226`). Required fields are `schema`, `runId`, `workflowName`, `startedAt`, `status`, `traceSchema`, and `paths{workflow,state,trace}` (all strings). Optional fields are `runTitle`, `workflowSource`, `finishedAt`, `paths.session`, `paths.artifacts`.
- `state`: serde `RunState` (`types.rs:228-298`).
  - Required: `schema`, `traceSeq` (u64), `runId`, `workflowName`, `startedAt`, `updatedAt`, `status`, `input` (any JSON, including `{}`), `outputs` (object), `results` (object), `steps` (array).
  - `input` is a `serde_json::Value`, so it cannot be omitted, but `null` is fine.
  - Optional: `runTitle`, `workflowSource`, `parentRunId`, `finishedAt`, `updates`, `currentNode`, `currentAttemptId`, `currentNodeStartedAt`, `statusDetail`, `humanDecision`, `paused`, `waitingOn`, `finalOutput`, `error`, and the settings fields.
- `display`: serde `WorkflowDisplay` (`types.rs:59-69`). Required: `status`, `controls` (array of strings), and `activity` and `reason` as nullable fields. Optional: `reasonContent`.
- `workflow`: serde `DefinitionSnapshot` (`types.rs:473-483`). Required: `schema`, `name`, `startAt`, `nodes` (object, order preserved), `edges` (array). The schema string is not checked.
- `session` must be an object. `merge_page` fails with "run view has no session object" otherwise (`client.rs:1315-1318`), and that error drops the connection.

Optional, all defaulted:
- `graphRevision`
- `graphSteps` (defaults to `state.steps`)
- `takenTransitions` (defaults to `[]`)
- `graphCursor` (defaults to `steps.len()-1`)
- `stepStart` (defaults to 0)
- `stepTotal` (defaults to `steps.len()`)
- `tracePage{start,total,items}`
- `session.binding`, `session.entryPage{start,total,items}`, `session.eventPage{...}`, `session.capture`, `session.integrity{malformed,tornTail}`, `session.replayCheckpoint`
- `settingsScopes`, `settingsStart`, `settingsTotal`
- `followUpQueue`, `followUpStart`, `followUpTotal`
- `updateStart`, `updateTotal`
- `live` (default false), `possiblyInterrupted` (default false)
- `graphHistory`
- `graphScene` (a precomputed layout; if absent piw computes `layout_graph`)

Not read: `schema` and `runId` in the payload, `runRevision`, `queue`, `updates`, `workflowMessages*`. Document structs do not use `deny_unknown_fields`, so extra keys are tolerated.

Avoid `graphHistory` in an adapter. When present it overrides `graphSteps` and `takenTransitions` (`client.rs:156-177`), which defeats step-scrubbing pages.

**Page request and response**

Page kinds (`protocol.rs:285-296`, `rename_all="snake_case"`): `steps, trace, trace_at_step, session_entries, session_events, settings, follow_ups, updates`. The TS server also supports `workflow_messages`, which piw never requests.

Receipt as read by `merge_page` (`client.rs:1259-1364`):
- `schema:"pi-workflows.run-page.v1"` (this is how the response is routed).
- `runId`, `kind`, `cursor` (u64), `revision` (u64) are all required.
- `start` and `total` default to 0.
- `items` is an array and is required.
- Per kind, it writes these keys into the stored view:

| kind | Extra receipt keys | Written into the view |
|---|---|---|
| `steps` | `graphSteps` (default: `items`), `graphCursor` (default 0), `takenTransitions` (default `[]`) | `graphSteps`, `stepStart`, `stepTotal`, `graphCursor`, `takenTransitions`, and `state.steps = items` |
| `trace`, `trace_at_step` | none | `tracePage = {start,total,items}` |
| `session_entries` | none | `session.entryPage` |
| `session_events` | `replayCheckpoint` (default null) | `session.eventPage`, `session.replayCheckpoint` |
| `settings` | none | `settingsScopes`, `settingsStart`, `settingsTotal` |
| `follow_ups` | none | `followUpQueue.items`, `followUpStart`, `followUpTotal` |
| `updates` | none | `updates`, `updateStart`, `updateTotal`, `state.updates` |

How the UI uses each kind:
- **steps and trace_at_step**: requested together with the same cursor when the replay position is not covered (`ui/mod.rs:219-231,725-745`). `step_projection_contains` (`ui/mod.rs:2739-2748`) requires `graphCursor == cursor`, plus `cursor` within `[stepStart, stepStart+len)`.
- **Plain `trace`**: never requested. The only `request_window` call sites pass `trace=None` (`ui/mod.rs:743,763,800`).
- **session_entries and session_events**: requested together, and only in temporal replay mode, which exists only if the snapshot has session events. If the snapshot has entries and no events, they are never requested.
- **settings, follow_ups, updates**: requested only from the info page keys `<` and `>`.

TS semantics for the steps page (`server.ts:5363-5433`, `store.ts:3666-3701`):
- `items` is the `state.steps` window of at most 256 steps.
- `graphSteps` has one compact step per node: the latest step with `step_index <= cursor`, ordered by index, with `prompt:null` and `output:null`.
- `takenTransitions` lists the distinct `"prevNode->node"` strings for steps `<= cursor`.
- `graphCursor` is the cursor, clamped to `total-1`.
- A TS page window is centered on the cursor, at most 256 items, via `workflowPageStart` (`view.ts:2027-2032`). An adapter with at most 256 steps can use `start:0`.

**Items shape per kind**
- **steps**: `StepRecord` (`types.rs:321-355`).
  - Required: `attemptId`, `nodeId`, `nodeType` (strings); `outcome` in `ok|timed_out|failed|cancelled`; `startedAt`, `finishedAt` (RFC 3339, parsed with `DateTime::parse_from_rfc3339`, `format.rs:34-38`); `prompt` and `output` (any JSON, null allowed, key must be present).
  - Optional: `error`, `action{actionType,command?,args?,cwd?,exitCode?,signal?,durationMs?}`, `assistantMessage`, `conversation{firstEntryId,lastEntryId}`, `settingsScopeId`, `settingsChangeNumber`, `settingsHash`.
  - A malformed `graphSteps` silently falls back to `state.steps` (`client.rs:160-168`). A malformed `state.steps` makes the whole snapshot Invalid.
- **trace and trace_at_step**: free-form JSON objects, never struct-parsed. Fields read:
  - `seq` (u64) and `type` (string).
  - `nodeId` and `attemptId` (strings).
  - `payload` (any JSON).
  - `at` (RFC 3339), used only for `update_published` progress events (`ui/mod.rs:3898-3926`).
  - The trace pane filters by `attemptId == selectedStep.attemptId`, so trace items need `attemptId`. Styling is keyed on `type` values `node_failed|run_failed|run_completed|node_started`.
- **settings, follow_ups, updates**: opaque JSON objects. Cosmetic fields read:
  - settings: `mountPath`, `invocation`, `changeNumber`.
  - follow-ups: `order`, `state`; the queue also has `presentationState`.
  - updates: only the count is used.
- **session_entries and session_events**: see section 5.

## 4. Run list and graph

**Run list delivery**
- After `view.runs.watch` the server sends a `runs` event. The payload is the page: `{schema:"pi-workflows.run-list-page.v1", revision:<string>, start:0, total:<n>, items:[...]}` (`client.rs:1074-1113`).
- Required: `revision` (string), `start == 0`, `total` (u64), `items` (array with `len <= total`). The `schema` field is not checked on the event.
- If `items.len() == total`, the list is replaced at once. Otherwise piw sends `view.runs.page {revision,cursor:items.len()}`.
- Page responses must be `accepted` with `receipt.schema:"pi-workflows.run-list-page.v1"` and the same `revision` and `total`, `start` equal to the current count, and non-empty `items` (`client.rs:1115-1162`). A stale revision is ignored. `conflict` clears the request.
- The adapter can avoid paging entirely by sending every run in one `runs` event.
- Send a new `runs` event whenever the list changes. Order is as sent.
- TS builds it in `src/server/view.ts:142-190`, at most 256 items per page, with revision `"<n>:<m>"`.

**Run row** (`ui/mod.rs:136-156`)
- Required: `manifest` (full `Manifest`, same as above) and `display` (`WorkflowDisplay`). Rows missing either are silently dropped.
- Optional: `live` (bool), `possiblyInterrupted` (bool).
- piw ignores the other TS fields `runId`, `workflowName`, `originSessionId`, `createdAt`, `updatedAt`. They are harmless to send.
- Shown: `manifest.runTitle` or `workflowName`, a glyph from `display.status`, and elapsed time. Elapsed runs to `finishedAt` for non-running states and to now for `running` (`ui/mod.rs:2131-2137`).

**Enum strings**
- `RunStatus` (`types.rs:15-27`, snake_case): `queued, running, waiting, paused, completed, failed, timed_out, cancelled, ambiguous`. The same set is used for `manifest.status`, `state.status` and `display.status`.
- `NodeOutcome` (`types.rs:153-160`): `ok, timed_out, failed, cancelled`.
- `WorkflowActivity` (`types.rs:52-57`): `supervised_runner, origin_turn`, or null.
- `WorkflowSource` (`types.rs:162-167`, `tag="kind"`, snake_case): `{"kind":"builtin","id","revision"}` or `{"kind":"file","path","hash"}`.
- `display.controls` is free-form strings (`types.rs:63-65`). Known values are `pause, resume, cancel, answer, human-answer, update, submit, review`.

**Graph**
- `workflow.nodes` is a map `nodeId -> object`. `nodeType` is read as `agent|compute|notify|action|checkpoint`; any other value renders as `?`.
- Other node keys read (`render.rs:100-134,281-396`, `types.rs:485-503`): `actionExecution` ("shell" gives `$`), `statusDetail`, `summary`, `humanDecision{audience,choices}`, `expectedOutput.kind`, `mountPath`, `localNodeId`, `includeTransition`.
- `workflow.edges` is an array of `EdgeDef` (`types.rs:505-518`, untagged): `{"from","to"}` or `{"from","switch":{"on":<string>,"cases":{<label>:<targetNodeId>}}}`.
- `startAt` is the start node. Edge ids are derived by piw (`layout.rs:63-90`).
- Node status is not stored per node. It is derived (`render.rs:211-237`):
  - Active (`running` display status): `display.status == running` and `state.currentNode == node`. Or `display.activity == origin_turn` and `state.waitingOn == node`.
  - Waiting: `state.waitingOn == node`. Both of these apply only at the latest position.
  - Otherwise: the outcome of the node's latest step in `graphSteps`, mapped `ok` to completed, `failed`, `timed_out`, `cancelled`.
  - A node with no step is queued.
- Taken-edge highlighting uses `takenTransitions` strings `"from->to"` (`render.rs:654-656,1131`).
- Graph rendering waits on `graph_projection_ready` (`ui/mod.rs:2351`): `graphCursor == stepTotal-1` at latest. Otherwise the title shows "LOADING REPLAY" and no graph draws.

## 5. Session entries and events (conversation)

**Gating**
- The Conversation tab does not check `session.binding`. `session_bound` (`ui/mod.rs:125-130`) only affects the Info tab and the capture flags.
- If the binding is valid (`schema == "pi-workflows.session-binding.v1"`) but `capture` is null, `assess_capture` returns "invalid" (`session.rs:220-231`). The graph title then shows "CAPTURE INVALID".
- With `binding:null`, capture status is "unavailable" and nothing is flagged.

**Entries mode (no events), simplest** (`conversation.rs:157-258`)
- Used when `eventPage.items` is empty.
- Each entry is `{"seq":<u64>,"at":<string>,"entry":{...}}` (`SessionEntryRecord`, `types.rs:413-418`). `seq`, `at`, `entry` are required.
- The `entry` object:
  - `entry.type == "message"`, `entry.id` (string), `entry.message.role`, and `entry.message.content`.
  - Role styling: `user`, `assistant`, `toolResult` or `tool`. Any other role is muted.
  - For a non-"message" `entry.type`, the type is used as the role and the whole entry JSON is the text.
- `content` is a string, or an array of parts (`conversation.rs:41-98`):
  - Text: `{"type":"text","text":..}`.
  - Thinking: `{"type":"thinking","thinking":..}`. A `text` field is also accepted.
  - Tool call: `{"type":"toolCall"|"tool_use"|"toolUse","id","name"|"toolName","arguments"|"input"}`. Rendered `[tool call: name] <args preview>`.
  - Tool result: part types `toolResult` or `tool_result`, with `content` or `output`, and `isError` or `is_error`. This is rendered `[tool result: completed|failed]: ...`.
  - For Pi's own tool results, use a message with `role:"toolResult"` and text parts.
- Per-step highlight and progressive reveal use `step.conversation.firstEntryId/lastEntryId`, matched against `entry.id`. Without a `conversation` range on any step, scrubbing back shows zero entries, but the latest view shows all (`conversation.rs:191-201`).
- If `entryPage.total != items.len()` (or likewise for events), integrity shows "paged". Any malformed entry or event gives "invalid temporal session record" (`ui/mod.rs:3622-3653`).
- Entries are never paged in this mode, so an adapter must keep them inside the 1 MiB frame.

**Temporal mode (events non-empty)** (`conversation.rs:269-311`, `session.rs`)
- Strict `SessionEventRecord` (`types.rs:420-437`). Required: `seq` (u64), `at` (string), `nodeId`, `attemptId`, `type`, `payload`. Optional: `turnId`, `messageId`, `toolCallId`.
- All of the `Event types` below must satisfy the integrity checks in `session.rs:66-209`. An event must have non-empty `turnId` if its type is known.
- The sequence must be contiguous from 1, or integrity gives "invalid".
- If integrity checks fail, the CAPTURE INVALID flag shows. Conversation text still renders.

Event types, with the minimum payloads (fixture `fixtures/session-events/normal.json`):

| `type` | Needs | `payload` |
|---|---|---|
| `turn_started` | `turnId` | any object |
| `turn_finished` | `turnId` | any object |
| `message_started` | `turnId`, `messageId` | `{role}` |
| `assistant_event` | `turnId`, `messageId` | `text_start / thinking_start / toolcall_start {contentIndex}`; `text_delta / thinking_delta / toolcall_delta {contentIndex, delta}`; `text_end / thinking_end {contentIndex, content}`; `toolcall_end {contentIndex, toolCall}`; `done`; `error` |
| `tool_execution_started` | `turnId`, `messageId`, `toolCallId` | `{toolName, args}` |
| `tool_execution_updated` | `toolCallId` | any object |
| `tool_execution_finished` | `toolCallId` | `{toolName, isError, result}` |
| `message_finished` | `turnId`, `messageId` | `{role, settled, entryId?}`. If `settled` is true, `entryId` must match an entry's `entry.id`. |

- A settled message uses the verbatim entry. Otherwise blocks from the `assistant_event`s are shown.
- Minimal temporal content for the five cases:

| Case | How |
|---|---|
| user message | `message_started{role:"user"}`, then `message_finished{settled:true,entryId}` with an entry role `user`; or `assistant_event text_end` with `content`. |
| assistant text | `assistant_event text_start / text_end{contentIndex,content}`. |
| thinking | `thinking_start / thinking_end{contentIndex,content}`. |
| tool call | `toolcall_start`, `toolcall_end{contentIndex,toolCall}`, plus `tool_execution_started`. |
| tool result | `tool_execution_finished{toolName,isError,result}`. |

- The `eventPage` also needs `session.capture` if `binding` is set: `{schema:"pi-workflows.session-capture.v1", eventSchema:"pi-workflows.session-event.v1", status:"recording|complete|failed", eventCount, entryCount, lastEventSeq}`, with counts matching for non-recording status (`session.rs:211-289`).
- Temporal mode also enables the event-by-event timeline. `stepIndex` on an event (optional) maps an event to a step (`ui/mod.rs:829-846`).

## 6. Start, cancel, resume, human decisions

- **Start/rerun**: piw cannot start or rerun a run. There is no `run.start` or restart anywhere in `tui/src`.
- **Pause/resume/cancel**: `p` toggles. It sends `run.resume` if `display.controls` contains `resume`, otherwise `run.pause` if it contains `pause` (`ui/mod.rs:1140-1153`, `run_command.rs:51-59`). `x` then `y` sends `run.cancel`, and only when `controls` contains `cancel` (`ui/mod.rs:1155-1170,1593-1595`, `run_command.rs:106-115`).
  - Request: `{operation:"run.pause|run.resume|run.cancel", runId, payload:{}, requestId, idempotencyKey (different from requestId), clientId, schema, type}`.
  - Response matched by `requestId`. Success is `accepted` or `adopted`; anything else shows an error notice with `error` (`client_commands.rs:75-85`, `ui/mod.rs:1179-1194`).
  - The UI state update comes from the next snapshot and run list, not from the response.
  - `controls` is taken from the run list row (`display.controls`). An adapter with no control channel can send `controls:[]`.
- **Human decisions/checkpoints**: not answerable from piw. It prints a notice that checkpoints must be answered inside the owning Pi session (`run_command.rs:61-75`). `answer`, `human-answer`, `update`, `submit` are display-only strings.

## 7. A fully decodable example

Fully decodable run snapshot payload (`protocol/fixtures/run-view-controls-v1.json:60-156`, `agentSnapshot`, trimmed to its keys):
```
{"schema":"pi-workflows.run-view.v1","runId":"run-agent-controls","revision":4,"runRevision":2,
 "manifest":{"schema":"pi-workflows.run-manifest.v1","runId":"run-agent-controls","workflowName":"agent-controls",
   "workflowSource":{"kind":"file","path":"/tmp/agent-controls.workflow.ts","hash":"abc"},
   "startedAt":"2026-01-01T00:00:00.000Z","finishedAt":null,"status":"waiting",
   "traceSchema":"pi-workflows.trace-event.v1","paths":{"workflow":"server","state":"server","trace":"server"}},
 "state":{"schema":"pi-workflows.run-state.v1","traceSeq":1,"runId":"run-agent-controls","workflowName":"agent-controls",
   "workflowSource":{...same...},"startedAt":"2026-01-01T00:00:00.000Z","finishedAt":null,
   "updatedAt":"2026-01-01T00:00:01.000Z","status":"waiting","input":{},"outputs":{},"results":{},"steps":[],"waitingOn":"work"},
 "workflow":{"schema":"pi-workflows.definition-snapshot.v1","name":"agent-controls","startAt":"work",
   "nodes":{"work":{"nodeType":"agent"}},"edges":[]},
 "display":{"status":"waiting","activity":null,"controls":["pause","cancel","update","submit"],
   "reason":"The workflow needs its assigned agent result."},
 "graphSteps":[],"takenTransitions":[],"graphCursor":0,"stepStart":0,"stepTotal":0,
 "tracePage":{"start":0,"total":0,"items":[]},
 "session":{"binding":null,"entryPage":{"start":0,"total":0,"items":[]},"eventPage":{"start":0,"total":0,"items":[]},"capture":null,"integrity":null},
 "settingsScopes":[],"settingsStart":0,"settingsTotal":0,"followUpQueue":null,"followUpStart":0,"followUpTotal":0,
 "updateStart":0,"updateTotal":0,"live":true,"possiblyInterrupted":false}
```
This fixture is used by the test at `client.rs:1489-1499` and checks the decode path.

## 8. Smallest viable adapter and risks

**Minimum messages**
1. Accept the Unix socket connection. Tolerate a no-data probe connection. Write the hello line immediately with `packageVersion:"0.17.7"`.
2. On `view.runs.watch`: respond `accepted`, then send a `runs` event with all rows in one page (`items.length == total`, `start:0`, a revision string that changes when the list changes). Re-send on change.
3. On `view.run.watch`: respond `accepted` (or `notFound` with an `error`), then send a `run_snapshot` event with the full payload and `runId`. Re-send the whole payload on every change (live follow). If a run has more steps than the 256-item window, also implement `view.page` for `steps`. Otherwise keep to at most 256 steps.
4. On `view.run.unwatch`: respond `accepted` and drop the subscription.
5. On `view.page`: respond `accepted` with a receipt carrying `schema, runId, revision` (equal to the snapshot's `revision`), `kind`, `cursor` (echo it), `start`, `total`, `items`. For `steps` add `graphSteps`, `graphCursor` (equal to `cursor`), `takenTransitions`. Needed for step scrubbing. Skip it only if the viewer never leaves the latest step.
6. Reply `rejected` to any other operation. `run.pause`, `run.resume`, `run.cancel` are optional; if unsupported, send `controls:[]` and the keys are disabled.
7. Everything else (`view.runs.page`, `view.content`, the other page kinds) is optional. Inline all content to avoid `view.content`.

For a read-only browser, steps 1-4 plus an unchanging snapshot are enough. Conversation needs `session.entryPage` (entries mode), with `step.conversation` ranges for per-step highlight.

**Risks and hard-to-fake items**
- **Canonical JSON**: the biggest trap. Sorted keys, no whitespace, `1.0` must be `1`, floats in ECMAScript form, and the exact string escaping above. A single deviation drops the connection with no visible reason. Run each outgoing line through a re-serialize check.
- **Version**: exact `0.17.7` string. This needs updating whenever piw is upgraded.
- **Sizes**: 1 MiB per line, including the whole session. Entries are never paged in entries mode, so cap them or use `$artifact` references (needs `view.content` with base64 chunks, `bytes` and `sha256` that match exactly, `media_type` `application/json` or `text/plain`; `client.rs:394-432,1164-1250`).
- **`graphCursor`**: must equal `stepTotal-1` at latest, or the graph shows "LOADING REPLAY" (`ui/mod.rs:2351`). The default (omitted) is correct, so leave it out or set it to match.
- **`session` object**: must always be present as an object. Without it, any later `session_*` page response kills the connection (`client.rs:1315-1318`).
- **`input`, `prompt`, `output` keys**: required keys, null allowed. Missing keys make the snapshot Invalid.
- **Timestamps**: must be RFC 3339 (`2026-01-01T00:00:00.000Z`) or durations and replay fail silently.
- **Version of status strings**: `timed_out`, not `timedOut`. Wrong enum strings make the snapshot Invalid, or silently drop a row.
- **Row drop**: a run row missing `manifest` or `display` is dropped silently, with no error.
- **`graphHistory`**: omit it. If present it overrides paged `graphSteps`.
- **Event-less binding**: if you set a valid `session.binding` you must also supply `capture` with matching counts, or the graph is flagged CAPTURE INVALID.
- **Hashes, sequence numbers, signatures**: none are required, except `$artifact` content digests (avoidable) and the page `revision` equality with `payload.revision`. Revisions only need to be consistent and monotonic.
- **ID limits**: all ids are 1 to 256 bytes.
- **Socket collision**: if the real pi-workflows server uses the default path, set `PIW_SOCKET` to the adapter's socket and `PIW_NO_AUTOSTART=1`, so piw never launches `pi-workflows server start`.
- **Headless check**: `piw --once <runId>` (`main.rs:31-36,195-201`, `ui/mod.rs:503-535`) renders one screen as plain text after waiting up to 10 s for a snapshot. It is the quickest verification path.
