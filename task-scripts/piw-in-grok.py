#!/usr/bin/env python3
"""Slices for step 3: pi-workflows runs started, streamed and answered inside Grok Build.

Method: skills/parallel-slices/SKILL.md in the pi-workflows repo. Each slice is one tool-less chat-completion request
that gets a contract plus WHOLE context files (no per-file cut) and returns complete new files as text. The coordinator
wires them into shared files by hand.

  python3 task-scripts/piw-in-grok.py --dry-run              # sizes, cost estimate, full prompts written to _prompts/
  python3 task-scripts/piw-in-grok.py --only activity        # fire one slice (pilot)

Key: OPENROUTER_API_KEY from the environment. It is never printed.
"""
import argparse, json, os, re, sys, time, urllib.error, urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any, Dict, List

HERE = Path(__file__).resolve().parent
PIW = Path.home() / "repos" / "pi-workflows"
GROK = HERE.parent
PI_DOCS = "node_modules/@earendil-works/pi-coding-agent/docs"

MODEL = "google/gemini-4-neon"  # OpenRouter id
ENDPOINT = "https://openrouter.ai/api/v1/chat/completions"
MAX_TOKENS = 64000
TIMEOUT, MAX_RETRIES, RETRY_DELAY = 900, 3, 2.0
PRICE_IN_PER_M, PRICE_OUT_PER_M = 3.00, 15.00  # google/gemini-4-neon on OpenRouter (checked 2026-11-21); estimate only
TaskSpec = Dict[str, Any]

SYSTEM = (
    "You are a principal software engineer. Write production-ready code with complete types and comprehensive tests. "
    "Read the attached repository files as the architecture you are extending and follow their conventions exactly. "
    "Return only the files asked for, in the return format given."
)

TS_RULES = """
Environment: TypeScript 7, `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`
(type-only imports use `import type` or inline `type`), module NodeNext (relative imports end in `.js`), target ES2023,
Node 22+. Tests are vitest 4 (`import { describe, expect, it, vi } from "vitest"`), live in `test/`, import from
`../src/...js`, and must not touch the network, real models, real child processes, or files outside temp directories.
Lint is oxlint; format is oxfmt (2 spaces, double quotes, semicolons, trailing commas, 100 columns, imports sorted).
Repository rules that bind this slice (from AGENTS.md and slophammer.yml, both attached):
- Layer imports: `src/state` imports no other layer. `src/server` may import `src/state`, `src/client`,
  `src/workflows`. `src/client` may import only `src/state`.
- Cyclomatic complexity of every function is at most 8. Split functions rather than nest branches.
- No duplicated code blocks: the dry check flags any copied block of 100 tokens or more, with zero findings allowed,
  so do not copy helper bodies from the attached files or repeat near-identical blocks between your own files.
- Coverage: the 85% thresholds count `src/state` (and not `src/server`), but test every exported item of both files.
- Structured values use camelCase fields and a versioned `schema` identifier.
- Do not add arbitrary limits to user-visible output. A limit is allowed only where a named external interface
  requires it; say which in a comment and keep the complete result available.
- No new dependencies. No `any`. No non-null `!` on values that come from outside the process.
Names in the "Public surface" blocks are fixed: other files are written against them. You may add private helpers
and extra exported items. Do not edit or re-declare anything in the attached files; the coordinator wires this module
into them by hand.
"""

MULTI_FILE_RETURN = """
Return format: every file complete, nothing omitted, no prose before, between or after. Each file is introduced by
a line of exactly this form, followed by one fenced block:

=== FILE: <repository-relative path> ===
```ts
<complete file>
```
"""

ACTIVITY_PROMPT = f"""
Slice id: activity
Goal: give pi-workflows a way to carry the live activity of a running agent step (the text the agent is writing,
its thinking, its tool calls, its status lines) from the executor that runs the step to any client watching the run.
Today this activity is thrown away in two places you can see in the attached files:
- `src/server/acpx-executor.ts`, `finishTurn`: `for await (const event of turn.events) void event;`
- `src/server/rpc-executor.ts`: the Pi child's stdout (the Pi RPC event stream) is kept only as an unread 64 KiB tail.
Clients get whole-run snapshots (`WorkflowRunView` in `src/client/view.ts`) pushed on a 250 ms timer by
`src/server/server.ts` (`publishSubscription`), and those snapshots have no field for streaming text.

How the pieces sit (read `src/server/server.ts`, `src/server/workflow-runner-protocol.ts` and
`src/client/protocol.ts` to see the surrounding architecture):
- Steps run in a separate runner process. The executor lives in the runner. The runner talks to the server in NDJSON
  messages, each at most 1 MiB, and every ordinary runner message is recorded durably and revision-checked.
- Live activity must NOT be durable and must NOT bump revisions. The coordinator will add one side-channel runner
  operation for it (like `process.register`, which skips the revision check) and one new client event. Both carry
  the values this slice defines. You do not write that wiring.
- So this slice is three things: (1) the shared value types and their strict parser, in the `src/state` layer so the
  client, the server and the engine can all import them; (2) the runner-side pieces: mappers from the two real event
  sources into those values, and a batcher that turns a token-by-token stream into a few messages per second, each
  safely under the frame limit; (3) the server-side hub that numbers activity per run, keeps it in memory for late
  subscribers, and fans it out.

Ownership: return exactly these three new files and nothing else:
- `src/state/step-activity.ts`
- `src/server/step-activity.ts`
- `test/step-activity.test.ts`
{TS_RULES}
Public surface of `src/state/step-activity.ts` (imports only `./json.js`):

```ts
export const STEP_ACTIVITY_SCHEMA = "pi-workflows.step-activity.v1" as const;
export const STEP_ACTIVITY_KINDS = ["text", "thought", "tool", "status"] as const;
export type StepActivityKind = (typeof STEP_ACTIVITY_KINDS)[number];

/** State of one tool call. Sent again, whole, each time the call changes. */
export interface StepActivityTool {{
  toolCallId: string;
  /** Short name of the tool or of the call, for a one-line row. */
  title: string;
  /** "pending" | "in_progress" | "completed" | "failed" when the source says; absent when it does not. */
  status?: "pending" | "in_progress" | "completed" | "failed";
  /** The source's own category word (acpx `kind`, e.g. "read", "edit", "execute"; Pi: the tool name). */
  kind?: string;
  input?: JsonValue;
  output?: JsonValue;
  /** True when `input` or `output` was shortened to fit a protocol frame (see the batcher). */
  truncated?: boolean;
}}

/** What an executor reports while a step runs. Never persisted. */
export interface StepActivityBody {{
  kind: StepActivityKind;
  /**
   * kind "text":    a chunk of the agent's answer, to be APPENDED to the previous "text" chunks of the attempt.
   * kind "thought": a chunk of the agent's thinking, appended the same way.
   * kind "status":  one complete status line; replaces the previous status line of the attempt.
   * kind "tool":    a one-line human description of the call (may be ""), with `tool` set.
   */
  text: string;
  /** Present exactly when kind is "tool". */
  tool?: StepActivityTool;
}}

export interface StepActivityKey {{
  runId: string;
  nodeId: string;
  attemptId: string;
}}

/** One numbered activity record as clients receive it. */
export interface StepActivity extends StepActivityBody, StepActivityKey {{
  schema: typeof STEP_ACTIVITY_SCHEMA;
  /** Per run, starts at 1, increases by exactly 1 with every record of the run, across all its attempts. */
  seq: number;
  /** ISO-8601 UTC time the server numbered the record. */
  at: string;
}}

/** Strict: exact keys, known kind, `tool` present iff kind is "tool", seq a positive safe integer. Throws Error. */
export function parseStepActivity(value: unknown): StepActivity;
/** Strict in the same way. Throws Error. */
export function parseStepActivityBody(value: unknown): StepActivityBody;
/** Array of bodies, as the runner sends a batch. Throws Error on a non-array or any bad element. */
export function parseStepActivityBodies(value: unknown): StepActivityBody[];
```

Parser rules. Messages are clear `Error`s naming the field, in the manner of the parsers in `src/client/protocol.ts`,
but the helpers there are private to another layer: write your own small ones in this file. Unlike
`requireExactKeys` there, these parsers reject BOTH unknown keys and missing required keys. Field rules: `runId`,
`nodeId`, `attemptId`, `at` and `tool.toolCallId` are non-empty strings (`at` is not otherwise checked); `text` and
`tool.title` are strings and may be empty for every kind; `tool.status` is one of the four words; `tool.kind` a
string; `tool.input` / `tool.output` any JSON value; `tool.truncated` a boolean. No length limits. Optional fields
that are absent stay absent (never `undefined`-valued keys), so `canonicalJson` of a parsed value equals
`canonicalJson` of its input.

Public surface of `src/server/step-activity.ts`:

```ts
import type {{ AcpRuntimeEvent }} from "acpx/runtime";   // same import path `src/server/acpx-executor.ts` uses
import type {{ StepActivity, StepActivityBody, StepActivityKey }} from "../state/step-activity.js";

/** acpx event of a running turn -> activity, or undefined for events that carry nothing to show. */
export function stepActivityFromAcpx(event: AcpRuntimeEvent): StepActivityBody | undefined;

/** One parsed record of Pi's RPC stdout stream -> activity, or undefined. Never throws, whatever the input. */
export function stepActivityFromPiRpc(record: unknown): StepActivityBody | undefined;

/** Splits a byte stream into JSON records on LF only (see Pi's rpc.md "Framing"), tolerating CR before LF. */
export class JsonlRecordReader {{
  /** Feed a chunk; returns the complete records it finished. A line that is not valid JSON is skipped. */
  push(chunk: Buffer | string): unknown[];
  /** The stream ended: returns the final unterminated line as a record if it is valid JSON, else []. */
  end(): unknown[];
}}

export interface StepActivityBatcherOptions {{
  /** Receives each batch. Every batch satisfies: Buffer.byteLength(canonicalJson(batch)) <= maxBatchBytes. */
  emit: (batch: StepActivityBody[]) => void;
  /** Longest time a pushed body waits before it is emitted. Default 100. */
  flushMs?: number;
  /**
   * Default 256 * 1024: a quarter of the 1 MiB runner and client frame limit, leaving room for the envelope.
   * Values below 1024 are raised to 1024, so one body with a short text always fits.
   */
  maxBatchBytes?: number;
  /** Test seams; default to the global timers. */
  setTimer?: (run: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
}}

/** Runner side. Coalesces a token-by-token stream into few, frame-safe batches without losing or reordering text. */
export class StepActivityBatcher {{
  constructor(options: StepActivityBatcherOptions);
  push(body: StepActivityBody): void;
  /** Emit whatever is waiting now. */
  flush(): void;
  /** flush(), then ignore every later push. */
  close(): void;
}}

export type StepActivityListener = (activities: StepActivity[]) => void;

/** Server side. One per server. Numbers, remembers and fans out activity. Holds no durable state. */
export class StepActivityHub {{
  constructor(options?: {{ now?: () => Date }});
  /** Number the bodies (seq, at, schema, key), remember them, hand them to the run's listeners, return them. */
  publish(key: StepActivityKey, bodies: StepActivityBody[]): StepActivity[];
  /** Everything remembered for the run with seq > afterSeq, in seq order. Unknown run: []. */
  read(runId: string, afterSeq: number): StepActivity[];
  /**
   * Calls `listener` synchronously, once, with read(runId, afterSeq) if that is non-empty, then with every later
   * publish of the run, until the returned function is called. Calling it twice is harmless.
   */
  subscribe(runId: string, afterSeq: number, listener: StepActivityListener): () => void;
  /** The seq of the newest record of the run, 0 if none. */
  lastSeq(runId: string): number;
  /** The run is over or was removed: forget its records and its listeners. Later publishes start again at seq 1. */
  closeRun(runId: string): void;
  /**
   * Where a watch asking for `afterSeq` really starts. When `afterSeq` is greater than lastSeq(runId) the hub's
   * numbering does not continue the client's (the server restarted): `{{ afterSeq: 0, reset: true }}`, and the
   * client is told to discard what it holds. Otherwise `{{ afterSeq, reset: false }}`.
   */
  resumePoint(runId: string, afterSeq: number): {{ afterSeq: number; reset: boolean }};
}}

/**
 * Split records into groups so each group's canonical JSON is at most maxBytes, keeping order, never splitting a
 * record, never returning an empty group. A single record larger than maxBytes gets a group of its own.
 */
export function stepActivityFrames(activities: StepActivity[], maxBytes: number): StepActivity[][];
```

Behaviour.

Mapping acpx (`AcpRuntimeEvent`, declared in the attached `node_modules/acpx/dist/runtime.d.ts`):
- `text_delta` with `stream` "thought" -> kind "thought"; any other `text_delta` -> kind "text"; `text` verbatim.
  An empty `text` -> undefined.
- `status` -> kind "status" with its `text`; empty text -> undefined. acpx also uses `status` events to carry
  usage updates, available-command lists and plans (see the `tag` values and the extra fields on the `status`
  variant in `runtime.d.ts`): those are not status lines; map them to undefined.
- `tool_call` -> kind "tool": `tool.toolCallId` from `toolCallId` (when the event has none, use its `title`, and when
  that is missing too, the literal "tool"); `tool.title` from `title`, else the event's `text`, else "tool";
  `status`, `kind` copied when they are one of the allowed words / a string; `input` from `rawInput`, `output` from
  `rawOutput`, each only when it is JSON-representable (run it through a JSON round trip; drop it if that throws).
  Body `text` is the event's `text`.
- `done`, `error` and anything else -> undefined.

Mapping Pi RPC records (shapes in the attached `json.md`; note wire `message_update` records are delta-only):
- `{{"type":"message_update","assistantMessageEvent":{{"type":"text_delta","delta":…}}}}` -> kind "text".
- same with `"thinking_delta"` -> kind "thought".
- `tool_execution_start` -> kind "tool", status "in_progress", `toolCallId`, title and kind = `toolName`,
  input = `args`.
- `tool_execution_update` -> the same with output = `partialResult`.
- `tool_execution_end` -> status "failed" when `isError` is true, else "completed"; output = `result`.
- Everything else (responses, `toolcall_delta`, `agent_end`, extension UI records, non-objects, records with fields
  of the wrong type) -> undefined. The body `text` of a tool record is "".

Batcher:
- Consecutive pushes of kind "text" are merged into one body by concatenating `text`; the same for "thought".
  A push of a different kind ends the merge (order of bodies across kinds is preserved exactly).
- A "status" push replaces a waiting "status" body only if that body is the last one waiting; otherwise it is
  appended.
- A "tool" push replaces a waiting "tool" body with the same `toolCallId` only if that body is the last one waiting;
  otherwise it is appended.
- The first push into an empty batcher arms one timer for `flushMs`; `flush` and `close` disarm it. No timer runs
  while nothing waits.
- Frame safety. No text is ever dropped: a "text" or "thought" body too large for one batch is split into several
  bodies at Unicode code point boundaries (never inside a surrogate pair) and spread over several `emit` calls, in
  order. A "tool" body too large for one batch gets `truncated: true` and is shortened in this order until it
  fits, measuring the real canonical JSON after each step: `output` becomes a string holding the leading characters
  of its canonical JSON, the kept length halved until the body fits or reaches 0; then `input` the same way; then
  `text`, then `tool.title`, then `tool.toolCallId` are cut the same way (toolCallId keeps at least one character).
  This is allowed because the frame limit is a named external interface and the complete tool result stays in the
  agent's own session record. When waiting bodies would exceed
  `maxBatchBytes`, emit before adding more.
- `emit` throwing must not break the batcher: catch, drop that one batch (the only case in which text is lost;
  the transport is gone and the step's result does not depend on activity), keep working.

Hub:
- Records are kept in memory per run, all of them, until `closeRun` (no cap: the coordinator calls `closeRun` when
  the run reaches a final state).
- A listener that throws is removed and does not stop delivery to the other listeners or make `publish` throw.
- `publish` with an empty array returns [] and notifies nobody.
- A listener added or removed during delivery does not disturb the delivery in progress.

Tests (`test/step-activity.test.ts`, one `describe` per exported item, names stating the observable behaviour).
Cover at least: every acpx mapping row including the fallbacks and a `rawInput` that cannot be JSON (a BigInt or a
cycle); every Pi mapping row and a handful of malformed records; the reader with a record split across chunks, CRLF,
a U+2028 inside a JSON string (must not split), an invalid line between two valid ones, a multi-byte character
split across two Buffer chunks, and `end` with a valid, an invalid and no unterminated line; the batcher's merge rules, the status and tool replacement rules including the "only
if last waiting" condition, timer arming and disarming through the seams, a 1,000,000-character text push with a
`maxBatchBytes` of 2048 whose emitted chunks concatenate back to the input exactly and each satisfy the byte bound, an
astral-plane character at a split point, the oversized tool body at each shortening stage, a `maxBatchBytes` below 1024, `close`, and a throwing `emit`; the hub's numbering
across two attempts of one run, isolation between runs, replay with `afterSeq`, live delivery, unsubscribe,
`closeRun` restarting at 1, `resumePoint` on both sides of lastSeq, a throwing listener; `stepActivityFrames` bounds and order; the parsers accepting a
round-tripped value and rejecting an extra key, a missing key, an unknown kind, `tool` on a "text" body, a missing
`tool` on a "tool" body, and seq 0 / 1.5 / a string.
{MULTI_FILE_RETURN}
"""

PAGER = "crates/codegen/xai-grok-pager/src"
BIN = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp"

WIRE = """
Live step activity on the wire. The coordinator is adding this to pi-workflows (server and `src/client/protocol.ts`);
treat it as existing even though the attached pi-workflows files do not have it yet:
- Request operation "view.run.activity.watch": envelope `runId` = the run; payload
  {"subscriptionId": <string chosen by the client>, "afterSeq": <integer >= 0>}. Answered like the other view watches.
- The server then sends events
  {"schema":"pi-workflows.client.v1","type":"event","subscriptionId":<id>,"event":"step_activity","runId":<run>,
   "payload":{"activities":[<StepActivity>, ...]}}
  first with everything already remembered after `afterSeq`, then live. Activities arrive in `seq` order; `seq` is
  per run, starts at 1 and increases by exactly 1. Each frame is under 1 MiB.
  The payload also carries `"reset": true` on the first event of a watch whose `afterSeq` is greater than the
  newest seq the server holds for the run (the server restarted and numbers again from 1): the client must then
  discard the activity it holds for that run before applying the event. `"reset"` is absent otherwise.
- Request operation "view.run.activity.unwatch": payload {"subscriptionId": <id>}.
- One StepActivity:
  {"schema":"pi-workflows.step-activity.v1","runId":…,"nodeId":…,"attemptId":…,"seq":<int>,"at":<ISO time>,
   "kind":"text"|"thought"|"tool"|"status","text":<string>,
   "tool":{"toolCallId":…,"title":…,"status"?:"pending"|"in_progress"|"completed"|"failed","kind"?:…,
           "input"?:<json>,"output"?:<json>,"truncated"?:<bool>}}      ("tool" present exactly when kind is "tool")
  kind "text": a chunk of the agent's answer, APPENDED to the earlier text chunks of that attempt.
  kind "thought": a chunk of the agent's thinking, appended the same way.
  kind "status": one complete status line; replaces the attempt's previous status line.
  kind "tool": the whole current state of one tool call, sent again each time the call changes.
  Activity is not durable: after a server restart or once a run is over the server may remember none of it.
  A server that does not know these two operations closes the connection without answering (see how
  `src/server/server.ts` treats a frame it cannot parse).
"""

VIEW_LIMITS = """
What a run view does and does not carry. The view is built by `readRun` in `src/server/view.ts` (attached whole;
it, not `server.ts`, is the authority) and typed in `src/client/view.ts`. Read both before relying on any field:
- `state.steps` is ONE byte-bounded page of full step records around the latest step, not every step. The complete
  list of attempts is `graphSteps`: compact records whose `prompt` and `output` are null.
- A step's `prompt` and `output` exist only on its FINISHED step record. The step that is running now is only
  `state.currentNode` / `state.currentAttemptId` / `state.currentNodeStartedAt`; it has no step record yet.
- Any value over 16 KiB (a prompt, an output) is replaced by a `$artifact` content reference; the text is fetched
  separately (`PiwClient::content`).
- For large workflow definitions the nodes in the view keep only a few fields; `agent`, `model` and the
  human-decision details may be missing. Treat every definition field as optional.
- The view has the session the run belongs to (`queue.originSessionId`) and timestamps (`queue.createdAt` /
  `startedAt` / `finishedAt`, each step's `startedAt` / `finishedAt`).
- The view does NOT carry the pending gate: the question's request id and revision are only in the SESSION
  snapshot's `interaction` (`ClientInteractiveRequest` in `src/client/view.ts`), which `PiwClient::watch_session`
  / `session` deliver. Use `gate_from` (below) rather than reading it yourself.
"""

PIW_CLIENT_API = """
```rust
// File crates/codegen/xai-grok-pager/src/piw/client.rs. Path inside the pager library: `crate::piw::client`;
// from the grok-acpx-acp binary: `xai_grok_pager::piw::client`.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind { Text, Thought, Tool, Status }

#[derive(Debug, Clone, PartialEq)]
pub struct ToolActivity {
    pub tool_call_id: String,
    pub title: String,
    pub status: Option<String>,   // "pending" | "in_progress" | "completed" | "failed"
    pub kind: Option<String>,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepActivity {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub seq: u64,
    pub at: String,
    pub kind: ActivityKind,
    pub text: String,
    pub tool: Option<ToolActivity>,   // Some exactly when kind == Tool
}
impl StepActivity {
    /// None for anything that is not a well-formed StepActivity.
    pub fn from_json(value: &Value) -> Option<StepActivity>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct PiwResponse {
    pub request_id: String,
    pub operation: String,
    pub run_id: Option<String>,
    pub outcome: String,          // the response's `outcome` word, verbatim
    pub revision: Option<u64>,
    pub receipt: Option<Value>,   // verbatim
    pub error: Option<Value>,     // verbatim
}
impl PiwResponse {
    /// True when the server accepted the request (see the outcome words in src/client/protocol.ts).
    pub fn ok(&self) -> bool;
    /// A one-line human message for a refusal, taken from `error`; None when ok().
    pub fn error_text(&self) -> Option<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState { Connecting, Connected, Reconnecting, Closed }

/// `PiwResponse::outcome` of a request the client gave up on locally because the connection dropped.
pub const CONNECTION_LOST: &str = "connection_lost";

#[derive(Debug, Clone, PartialEq)]
pub struct RunView {
    pub revision: u64,       // the view payload's `revision` (presentation cursor), 0 if absent
    pub run_revision: u64,   // the view payload's `runRevision` (execution revision), 0 if absent
    pub view: Value,         // the latest `run_snapshot` event payload: one complete WorkflowRunView (src/client/view.ts)
}

/// A value the server replaced by a `$artifact` content reference, fetched in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content { Pending, Ready(String), Invalid(String) }

/// Send + Sync. Share it with `Arc<PiwClient>`. Every method except `call` returns immediately and never does I/O
/// on the calling thread: a worker thread owns the socket, reconnects, and re-sends the watches after a reconnect.
pub struct PiwClient { /* private */ }
impl PiwClient {
    /// The server socket of this machine's default state folder, by the same rule as `src/client/protocol.ts`.
    pub fn default_socket_path() -> Option<PathBuf>;
    /// Returns at once; the worker connects in the background. Err only if the worker thread cannot be started.
    pub fn connect(socket: &Path) -> anyhow::Result<PiwClient>;
    pub fn state(&self) -> ConnState;
    pub fn last_error(&self) -> Option<String>;
    /// The `clientId` this client puts on every request.
    pub fn client_id(&self) -> String;
    /// The server's `packageVersion` from its hello, once connected.
    pub fn server_version(&self) -> Option<String>;
    /// How many times the connection has been established (1 after the first connect). The coordinator role and
    /// its epoch belong to one connection, so users of session commands compare this to notice a reconnect.
    pub fn connection_count(&self) -> u64;
    /// Bumps every time any cached list, view, session snapshot or activity changes. Lets pollers skip work.
    pub fn generation(&self) -> u64;

    pub fn watch_runs(&self);
    /// Latest run summaries (WorkflowRunSummary JSON values), in the server's order. Empty until the first event.
    pub fn runs(&self) -> Vec<Value>;
    pub fn watch_run(&self, run_id: &str);
    pub fn unwatch_run(&self, run_id: &str);
    pub fn run_view(&self, run_id: &str) -> Option<RunView>;
    /// Starts (once) the activity watch of the run from seq 0 and keeps everything received.
    pub fn watch_activity(&self, run_id: &str);
    /// Received activity of the run with seq > after_seq, in seq order.
    pub fn activity(&self, run_id: &str, after_seq: u64) -> Vec<StepActivity>;
    /// Watch a session's snapshot; with `coordinator` true the watch also claims the coordinator role.
    pub fn watch_session(&self, session_id: &str, coordinator: bool);
    /// Latest session snapshot payload (`interaction`, `coordinatorEpoch`, `coordinatorActive`,
    /// `branchReportRequired`, ...), verbatim. After a reconnect it is None until the new snapshot arrives.
    pub fn session(&self, session_id: &str) -> Option<Value>;
    /// End the session watch (and with it this connection's hold on the coordinator role, as the server defines).
    pub fn unwatch_session(&self, session_id: &str);
    /// The text behind a `$artifact` content reference found in a view of `run_id`: starts the fetch
    /// (`view.content`, as `piw` does) on first call and returns Pending until it is complete. Cached per reference.
    /// A value that is not a reference gives Invalid.
    pub fn content(&self, run_id: &str, reference: &Value) -> Content;

    /// Queue a request with a fresh idempotency key; returns its requestId. It is sent when connected (and kept
    /// until then).
    pub fn send(&self, operation: &str, run_id: Option<&str>, expected_revision: Option<u64>, payload: Value) -> String;
    /// The same with the caller's idempotency key, so a caller can repeat a request the server may already have
    /// applied (the server deduplicates on the key).
    pub fn send_keyed(&self, operation: &str, run_id: Option<&str>, expected_revision: Option<u64>, payload: Value, idempotency_key: &str) -> String;
    /// The response to a request, once; None while it has not arrived. A request that was on the wire when the
    /// connection dropped is NOT re-sent: it completes with `outcome == CONNECTION_LOST` (ok() false) and the
    /// caller decides whether to repeat it with `send_keyed`.
    pub fn take_response(&self, request_id: &str) -> Option<PiwResponse>;
    /// `send` and wait. Blocks the calling thread up to `timeout`; never call it on the UI thread.
    pub fn call(&self, operation: &str, run_id: Option<&str>, expected_revision: Option<u64>, payload: Value, timeout: Duration) -> anyhow::Result<PiwResponse>;
    /// Stop the worker and close the socket. Later calls are no-ops; `state()` is Closed.
    pub fn close(&self);
}
```
"""

PIW_GATE_API = """
```rust
// File crates/codegen/xai-grok-pager/src/piw/gate.rs (`crate::piw::gate`, `xai_grok_pager::piw::gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind { Decision, Checkpoint }

#[derive(Debug, Clone, PartialEq)]
pub struct PiwGateChoice {
    pub key: String,                 // the choice's key in the workflow definition; what the answer names
    pub label: String,               // what the user sees (the choice's label, else its key)
    pub text_input: Option<String>,  // name of the text input this choice declares, when it wants text with it
}

/// A run parked on a question for a human.
#[derive(Debug, Clone, PartialEq)]
pub struct PiwGate {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub session_id: String,   // the session the answer goes through (`targetSessionId` of the request)
    pub kind: GateKind,
    pub request_id: String,   // the pending interaction request the answer must name
    pub revision: u64,        // the INTERACTION's revision (sent as expectedRevision), not the run's
    pub question: String,
    pub choices: Vec<PiwGateChoice>,   // sorted by `key` (the wire is canonical JSON; author order is not kept).
                                       // Empty for a checkpoint, which takes a free JSON/text input instead.
}

/// The gate a run is waiting on: Some only when `session` (a session snapshot from `PiwClient::session`) has a
/// pending `interaction` of kind "decision" or "checkpoint" for the run of `view`.
pub fn gate_from(view: &crate::piw::client::RunView, session: &serde_json::Value) -> Option<PiwGate>;

/// Grok Build's existing gate plumbing carries a "flow file" string per choice. For pi-workflows gates that string
/// is an answer target: `piw-answer://<run id>/<choice key>` (both percent-encoded: every byte outside
/// [A-Za-z0-9._-] as %XX). The coordinator routes such targets to the answer path instead of starting a flow.
pub const ANSWER_SCHEME: &str = "piw-answer://";
pub fn answer_target(run_id: &str, choice_key: &str) -> String;
/// (run id, choice key) of an answer target; None for anything else (for example a real flow file path).
pub fn parse_answer_target(target: &str) -> Option<(String, String)>;
```
"""

RUST_RULES = """
Environment: Rust edition 2024, toolchain 1.94. No `unsafe`. No `unwrap()` / `expect()` / indexing that can panic on
anything read from the socket, the disk or the user (a poisoned mutex is recovered with `into_inner`, not unwrapped).
Edition 2024: `gen` is a reserved word; `std::env::set_var` is unsafe (do not use it, not even in tests).
Names in the "Public surface" blocks are fixed: other files are being written against them right now, by other
engineers who see only those blocks. You may add private helpers and extra public items. Do not re-declare a type
another block owns; import it by the path given. Do not edit the attached files: the coordinator adds the `mod`
lines and wires this file into the existing code by hand.
JSON from the server is read defensively as `serde_json::Value`: a missing or wrongly typed field makes that one
thing absent; it never panics and never discards the rest.
Tests: a `#[cfg(test)] mod tests` at the bottom of the file; no real pi-workflows server, no network, no real
agent, nothing outside `tempfile::tempdir()`.
Return format: the complete file, nothing omitted, no prose, as:

=== FILE: <repository-relative path> ===
```rust
<complete file>
```
"""

CLIENT_PROMPT = f"""
Slice id: client
Target file: `{PAGER}/piw/client.rs` (new, in the Grok Build pager library)
Goal: a client for the pi-workflows server's local socket that the Grok Build terminal UI and its helper binary can
use from synchronous code that must never block: a worker thread owns the connection; callers read cached state.

What exists. pi-workflows runs one server per machine on a Unix socket and speaks newline-delimited canonical JSON
(`src/client/protocol.ts`, `src/client/client.ts`, `src/server/server.ts`, all attached, are the authority for the
handshake, the envelopes, the operations, their payloads and the events). pi-workflows already ships a Rust watcher,
`piw`, whose client is attached whole (`tui/src/client.rs`, `tui/src/protocol.rs`, `tui/src/client_commands.rs`):
it is the working reference for the handshake, framing, canonical JSON, request ids, idempotency keys, reconnecting
and applying run events. Study it and port what applies; this file is smaller in scope: it keeps views as JSON values
instead of decoding them, has no WebSocket mode and no paging, and adds activity, session watches and arbitrary
requests. Differences from `piw` that matter:
- Do NOT port piw's check that the server's `packageVersion` equals the client's own crate version (this crate has
  an unrelated version). Accept any server whose hello has the right protocol schema; keep its `packageVersion`
  for `server_version()`.
- On a watch the server sends only whole events: `runs`, `run_snapshot`, `session_snapshot`, `unavailable`
  (`publishSubscription` in server.ts) plus the new `step_activity`. There are no patches or pages to apply.
- The socket path rule needs `workflowStatePath()` from `src/state/database.ts` (attached) and, like piw's
  `tui/src/main.rs` (attached), honours the `PIW_SOCKET` environment variable when it is set. Grok Build has no such client yet; `crates/codegen/pi-grok-adapter/src/subagent_transport.rs`
(attached) shows this repo's tokio Unix-socket style.
{WIRE}
Crates you may use (already dependencies of the pager library): tokio (full), serde, serde_json, anyhow, thiserror,
tracing, uuid (v4, v7), chrono, libc, base64 (0.22); dev: tempfile, tokio test-util. Nothing else: no sha2, no ryu-js.
Canonical JSON: the server rejects (by closing the connection) a frame that is not canonical. Requests this client
builds contain only strings, booleans, integers, null, arrays and objects (a payload holding a non-integer number is
refused locally with an error response, never sent), so canonical JSON is compact `serde_json` output with object
keys sorted by UTF-16 code units, as JavaScript sorts them and as `tui/src/protocol.rs` does (sort them yourself; do
not rely on a serde_json feature, and note Rust's default string order is by code point, which differs).
{RUST_RULES}
Public surface (implement exactly):
{PIW_CLIENT_API}
Behaviour:
- The worker is one `std::thread` running a current-thread tokio runtime (as `piw` does). It connects, does the
  handshake the server expects, then sends every active watch and every queued request. On a lost connection: state
  Reconnecting, back off (start 250 ms, double to a 5 s cap), reconnect, re-send every active watch (activity
  watches from the highest seq already held, so nothing is duplicated; session watches with the same `coordinator`
  flag, which gives this connection a NEW coordinator epoch). Run and runs views already cached stay readable while
  reconnecting; session snapshots are cleared on disconnect (their epoch died with the connection).
- Requests are never re-sent by the client. One that was written to the socket and not answered when the
  connection dropped completes with outcome `CONNECTION_LOST`. One still queued (never written) is sent after the
  reconnect.
- A frame that makes the server close the connection must not cause an endless reconnect loop. Track what was
  written last before each drop: a watch (by kind and target) that was the last thing written before two drops in
  a row is switched off and named in `last_error` (this is what happens to the activity watch on a server without
  the activity operations: the runs and run watches must keep working); a request in that position is completed
  with `CONNECTION_LOST` (it already is, by the rule above).
- `run_view` always returns the latest whole `run_snapshot` payload for the run. An `unavailable` event for a run
  clears its view and sets `last_error`.
- Activity: keep every received StepActivity per run in seq order; drop a record whose seq is not greater than the
  last held (a replay after reconnect). An event whose payload has `"reset": true` discards the held activity of
  that run first.
- Content: `content` sends `view.content` for the reference exactly as `piw` does (chunked, base64), assembles the
  text in the worker, and caches it by reference; a failed or malformed fetch gives `Invalid(reason)`.
- `unwatch_run` also ends the run's activity watch and frees its cached view and activity.
- Frames longer than the protocol's maximum, frames that are not JSON, and events of unknown kind are skipped and
  recorded in `last_error`; they never end the connection or panic.
- `call` returns Err on timeout or when the client is closed, with a message naming the operation.
- Dropping the last `PiwClient` closes it.

Tests. Stand up a fake server in the test (a `UnixListener` in a tempdir, on a thread or a test runtime, speaking
the protocol by hand). Cover: handshake then `watch_runs` delivering summaries; `watch_run` snapshot then an updated
snapshot; activity delivered in order, a duplicate dropped, a renumbered stream after a server restart; `send` /
`take_response` and `call` success, refusal (`error_text`) and timeout; a request sent while disconnected going out
after connect; reconnect re-sending the watches, completing the unanswered request with `CONNECTION_LOST` and
clearing the session snapshot; a watch that closes the connection twice being switched off while the other watches
keep delivering; `send_keyed` putting the caller's key on the wire; a server hello with another `packageVersion`
being accepted; a `reset` activity event; a content reference fetched in two chunks; a payload with a non-integer
number refused locally; an oversized frame, a non-JSON frame and an unknown event kind being skipped; `close`; `StepActivity::from_json` accepting each kind and
rejecting a missing field and a tool record without `tool`, and accepting `truncated` true, false and absent;
canonical key order of an emitted request, including two keys whose UTF-16 and code point orders differ.
"""

DRIVER_PROMPT = f"""
Slice id: driver
Target file: `{BIN}/piw_driver.rs` (new, in the binary `grok-acpx-acp`)
Goal: make a pi-workflows run show up in a Grok Build session exactly the way an acpx flow run does today, live.

What exists. `grok-acpx-acp` is the agent behind Grok Build's flow mode. Its `driver.rs` (attached whole) runs an
acpx flow and turns its run folder into an ordered stream of `DriverEvent`s; `project.rs` (attached whole) turns
those events into what the user sees: a block per agent step with the agent's text streaming into it, tool rows, a
plan of the steps, the gate question. `agent.rs` (attached whole) owns the loop. This slice is a second source of
the SAME `DriverEvent`s: instead of reading an acpx run folder it watches one run on the pi-workflows server through
`PiwClient` (another engineer is writing it now; its surface is below) and emits events from the run's view and its
live step activity. `project.rs` must need no change to display it, so read `Projector::on_event` and everything it
calls to learn precisely what it does with each event and each field, and in particular what shape of
`AgentUpdate.update` (an ACP `session/update` value, as acpx records them) it understands for agent text, agent
thoughts, tool calls and tool call updates. Build exactly those shapes from StepActivity.

The run view is `WorkflowRunView` JSON. `piw`'s decoder of the same views is attached (`tui/src/state/types.rs`,
`tui/src/client.rs`) as a worked example of reading them.
{VIEW_LIMITS}{WIRE}
Crates available in the binary: tokio (full), serde_json, anyhow, async_trait, tracing, libc (unix), xai_grok_pager
(only the items named here); dev: tempfile. No serde derive, no chrono.
{RUST_RULES}
Types you import, do not re-declare:
- From `crate::driver`: `DriverEvent`, `StepKind`, `FlowSketch`, `SketchNode`, `SketchEdge`, `GateChoice`, by the
  names `driver.rs` declares. Outcomes and statuses in its events are plain `String`s: use the same words
  `driver.rs` emits for acpx runs, which are the words `project.rs` and `agent.rs` test for.
- `use xai_grok_pager::piw::client::{{PiwClient, RunView, StepActivity, ActivityKind, ToolActivity, ConnState, Content}};`
{PIW_CLIENT_API}
- `use xai_grok_pager::piw::gate::{{gate_from, answer_target, PiwGate}};`
{PIW_GATE_API}
Public surface (implement exactly):

```rust
pub struct PiwRunHandle {{ /* private */ }}
impl PiwRunHandle {{
    /// Ask the server to cancel the run ("run.cancel") and stop watching once the run reports a final state.
    pub fn cancel(&self);
    /// Stop watching without touching the run.
    pub fn detach(&self);
}}

/// Watch `run_id` and send its events to `tx` until the run is final (RunFinished is the last event) or the handle
/// is detached. Spawns a tokio task with `tokio::task::spawn_local` (the caller runs inside a `LocalSet`, like
/// `driver::start`). `poll_ms` is how often the client's caches are checked (use `PiwClient::generation` to skip
/// idle ticks).
pub fn watch(
    client: std::sync::Arc<PiwClient>,
    run_id: &str,
    poll_ms: u64,
    tx: tokio::sync::mpsc::UnboundedSender<DriverEvent>,
) -> PiwRunHandle;

/// The pure core, for tests and for replaying a finished run: feed it each new view, each batch of activity and
/// the current gate; it returns the events to emit, in order. `watch` is a thin loop around it: it watches the run,
/// its activity and (without claiming the coordinator role) the run's session, and calls `gate_from`.
pub struct PiwTailer {{ /* private */ }}
impl PiwTailer {{
    pub fn new(run_id: &str) -> Self;
    /// `text_of` turns a view value into text: a JSON string as itself, another inline value as compact JSON, a
    /// `$artifact` reference through `PiwClient::content` (None while it is still Pending or when Invalid).
    pub fn on_view(&mut self, view: &RunView, text_of: &dyn Fn(&serde_json::Value) -> Option<String>) -> Vec<DriverEvent>;
    pub fn on_activity(&mut self, activity: &[StepActivity]) -> Vec<DriverEvent>;
    /// The gate the run is parked on now, or None. Tests build `PiwGate` values directly.
    pub fn on_gate(&mut self, gate: Option<&PiwGate>) -> Vec<DriverEvent>;
    pub fn finished(&self) -> bool;
}}
```

Behaviour:
- Event order is the contract `project.rs` relies on, the same as for acpx runs: `RunStarted` first (title and
  `FlowSketch` built from the view's workflow definition: nodes in the order a reader would walk them from the start
  node, edges with their labels; `run_dir` and `flow_path` get the best equivalents the view offers, else empty
  paths); for each step attempt `StepStarted`, then for agent steps the `AgentUpdate`s, then `StepFinished`;
  `Waiting` when the run parks on a human gate; `RunFinished` last.
- Which attempts exist comes from `graphSteps` (all of them, compact) plus the running one from
  `state.currentNode` / `currentAttemptId` / `currentNodeStartedAt`; prompts and outputs come from the full records
  in `state.steps` when the attempt is in that page.
- `AgentPrompt`: a prompt is known only once its step has finished, and `project.rs` draws it as the opening
  message of the step's block. Emit `AgentPrompt` only when the prompt text is available at the moment the
  attempt's `StepStarted` is emitted (a step first seen already finished); never emit it after an `AgentUpdate` of
  the same attempt, and never hold `StepStarted` back to wait for it.
- Map pi-workflows node kinds onto `StepKind` (agent -> Acp; action/compute/notify -> the closest of Shell, Compute
  or Other, judged by what `project.rs` draws for each; checkpoint and human decision -> Checkpoint).
- Activity for an attempt whose `StepStarted` has not been emitted yet is held until it has; activity for an
  attempt already finished is dropped. On `StepFinished` of an agent step first flush its held activity.
- Text is streamed, never re-sent: each "text" / "thought" activity becomes one update carrying only that chunk.
  A "tool" activity becomes a tool call the first time its `tool_call_id` is seen in the attempt and a tool call
  update after that. "status" activity becomes whatever `project.rs` already shows as an agent's status line, or is
  dropped if it shows none.
- If no activity ever arrives for an agent step (server restarted, or a finished run opened later), the step still
  shows its final text: on `StepFinished`, when no "text" activity was emitted for the attempt and `text_of` gives
  the step's output, emit that output once as agent text before `StepFinished`. If the output is a reference still
  being fetched, hold that step's `StepFinished` (and everything after it) until a later `on_view` call resolves
  it, or until three further `on_view` calls have passed, then finish without the text.
- `Waiting` is built in `on_gate`: `node` and `attempt` from the gate, `question`, and one `GateChoice` per choice
  with its `label` and `flow_file` = `answer_target(run_id, key)` (see the shared block: `agent.rs` shows and
  starts "flow files"; the coordinator routes answer targets to the gate answer instead). A checkpoint gate has no
  choices: emit `Waiting` with an empty list. Emit once per gate (per `request_id`); nothing for a repeat.
- Views arrive as whole snapshots at any pace and may skip intermediate states: derive events by comparing with what
  has already been emitted, never assuming one change per view. Every `at_ms` comes from the view's timestamps for
  that thing, else from the wall clock.
- The run's final status maps onto `RunFinished`'s status as `driver.rs` uses it for acpx runs (completed, failed,
  cancelled; anything unknown is a failure with the server's word in `error`).
- While `ConnState` is not Connected nothing is emitted and nothing fails; watching resumes by itself.

Tests (pure, through `PiwTailer`, with views written as `serde_json::json!` literals shaped as `readRun` builds
them): a three-step run delivered view by view; the same run delivered as one final view (the same `StepStarted` /
`StepFinished` / `RunFinished` sequence, with `AgentPrompt`s now present); a running step known only from
`state.currentNode`; an attempt present in `graphSteps` but outside the `state.steps` page; activity before its
step's view; text chunks staying separate and in order; a tool call then its update; a finished step with no
activity falling back to its output, inline and through a reference that resolves one view later; no `AgentPrompt`
after updates; a gate emitted once across repeated calls with answer targets as flow files, then the run
continuing after the answer; a checkpoint gate; failed and cancelled runs; a view with missing or wrongly typed fields producing no panic; the FlowSketch
node and edge order for a branching definition.
"""

MIRROR_PROMPT = f"""
Slice id: mirror
Target file: `{PAGER}/piw/mirror.rs` (new, in the Grok Build pager library)
Goal: let Grok Build's existing flow screens (the runs pop-up with its list, run view, step detail and inspector,
the side-panel list of runs) show pi-workflows runs next to acpx runs WITHOUT those screens changing.

Why a mirror. Those screens do not take run data in memory. They read acpx run folders from disk, synchronously, on
a one-second tick: `acpx/bundle.rs` (attached whole) lists and loads runs from a runs directory, step conversations
are read from session event files through `acpx/sessions.rs` (attached), agent settings come from `flow.json` and
`projections/steps.json` under the run folder (`node_settings`, `step_settings`), stdout / stderr / outputs are
artifact files (`read_artifact`), and the screens call these with `run.summary.dir` (`views/acpx_modal/run.rs`,
`detail.rs`, `ledger.rs`, `views/side_panel/flows.rs`, `app/dispatch/acpx_run.rs`, all attached whole). So the only
way to feed them unchanged is to give them a run folder. This slice keeps, for each pi-workflows run, a folder in
acpx's run-folder format under a mirror directory, written and updated from what a `PiwClient` (surface below;
another engineer is writing it now) has cached. The reading code in `bundle.rs` and `sessions.rs` is the exact
specification of the format: every file, field and event name they read, and how. The folder layout is also
described in prose in the `RUN_FOLDER` string of the attached `task-scripts/acpx-mode.py` (ignore the rest of that
file). Read the screens to see which fields actually reach the user (for example the trace event names the timeline
looks for in `run.rs` and `detail.rs`, how a waiting run's choices come from `bundle::gate_choices`, the 90 second
stall rule in `bundle::is_stalled`), and write exactly what makes them show a pi-workflows run correctly.

The source is `WorkflowRunSummary` / `WorkflowRunView` JSON. `piw`'s decoder of the same views
(`tui/src/state/types.rs`, `tui/src/client.rs`) is attached as a worked example.
{VIEW_LIMITS}{WIRE}
Crates: those of the pager library (serde, serde_json, anyhow, chrono, tracing); dev: tempfile.
{RUST_RULES}
Types you import, do not re-declare:
- `use crate::piw::client::{{PiwClient, RunView, StepActivity, ActivityKind, ToolActivity, Content}};`
{PIW_CLIENT_API}
- `use crate::piw::gate::{{gate_from, answer_target, PiwGate}};`
{PIW_GATE_API}
Public surface (implement exactly):

```rust
/// Everything known about one run at one moment; the pure input of the writer.
pub struct MirrorInput<'a> {{
    pub summary: Option<&'a serde_json::Value>,   // its WorkflowRunSummary, when the runs list has it
    pub view: Option<&'a RunView>,                // its run view, when the run is being watched
    pub activity: &'a [StepActivity],             // all activity received for it (may be empty)
    pub gate: Option<&'a PiwGate>,                // the gate it is parked on
    /// Text of a view value: a JSON string as itself, another inline value as compact JSON, a `$artifact`
    /// reference once fetched (None while pending).
    pub text_of: &'a dyn Fn(&serde_json::Value) -> Option<String>,
}}

/// The run folder name used for a pi-workflows run id (safe as one path component, reversible).
pub fn folder_name(run_id: &str) -> String;
/// The run id of a mirror folder, None for a folder this module did not write.
pub fn run_id_of_folder(dir: &std::path::Path) -> Option<String>;
/// True when `dir` is a run folder inside `mirror_root` (so callers can route answers and cancels).
pub fn is_mirror_dir(mirror_root: &std::path::Path, dir: &std::path::Path) -> bool;

/// Write or update the run's folder under `mirror_root` so that `bundle::list_runs(mirror_root)` and
/// `bundle::load_run` return the run as the screens should show it. Idempotent: writing the same input twice
/// changes no file. Each file is replaced atomically (temp file + rename in the same directory), so a reader on
/// another thread never sees a half-written file; append-only files (session events, the trace) are appended to.
pub fn write_run(mirror_root: &std::path::Path, input: &MirrorInput<'_>) -> anyhow::Result<std::path::PathBuf>;

/// Keeps the mirror current on its own thread: watches the runs list, mirrors every run's summary, and for runs
/// that are active or were asked for with `want_run` also watches the view, the activity and the session and
/// mirrors those. Never blocks the caller.
pub struct PiwMirror {{ /* private */ }}
impl PiwMirror {{
    pub fn start(client: std::sync::Arc<PiwClient>, mirror_root: std::path::PathBuf) -> anyhow::Result<PiwMirror>;
    pub fn root(&self) -> &std::path::Path;
    /// A screen opened this run: mirror it in full from now on (finished runs included).
    pub fn want_run(&self, run_id: &str);
    /// The screen closed it: a finished run stops being watched.
    pub fn release_run(&self, run_id: &str);
    /// The gate a run is parked on, for the answer path.
    pub fn gate(&self, run_id: &str) -> Option<PiwGate>;
    /// "connected" / "connecting" / "reconnecting" / the last error, for a status line.
    pub fn status_line(&self) -> String;
    pub fn stop(&self);
}}
```

Behaviour:
- Fidelity is judged through the reader: whatever `bundle.rs` returns for the mirrored folder is what the user
  sees. Fill every field the pi-workflows data can supply with the meaning `bundle.rs` and the screens give it:
  run status (map the pi-workflows display statuses onto the statuses acpx writes; a cancelled, paused or queued
  run becomes the closest acpx status, with the pi-workflows word in the status detail), title, flow name, started
  / updated / finished times, current node, what it is waiting on, the error; the flow definition with node kinds,
  edges and branch edges; per step attempt its kind, outcome, times, prompt, output, error, agent name, model and
  effort (in the places `node_settings` / `step_settings` read them).
- The step's conversation is written as session event records in the shape `sessions.rs` parses, built from the
  step's StepActivity: text chunks appended in order to the agent's message, thoughts likewise, tool calls with
  their latest state. With no activity for a finished step, its output (via `text_of`) is the agent's message. A
  running step shows its text so far and grows on each write. Activity may be gone after a server restart: never
  delete conversation records already written because a later write has less activity.
- A run parked on a gate is written the way acpx writes a run waiting at a checkpoint, with the question, and with
  the choices where `bundle::gate_choices` reads them: label -> `answer_target(run_id, key)` (the screens start the
  "flow file" of the chosen label; the coordinator routes answer targets to the gate answer). A checkpoint gate
  (no choices) is written as waiting with the question and no choices.
- Stall rule: the screens call a running run stalled when its updated time is older than 90 seconds. A
  pi-workflows agent step can run longer than that without the view changing, so while a run is running set the
  updated time from the newest of: the view's own timestamps, the newest activity `at`, and the time of this write
  if the client is connected.
- Trace events: write the events the run view's timeline and the detail screen look for, in time order, once each
  (a rewrite never duplicates an event).
- Missing or wrongly typed fields never panic and never drop the run: that field is left as acpx leaves it when it
  has nothing.
- `PiwMirror`: one `std::thread`, woken by `PiwClient::generation` changes (poll every 250 ms; write only when
  the generation moved or a running run's stall clock needs refreshing, at most once a second per run). Watches the
  session of each fully mirrored run WITHOUT claiming the coordinator role, to get its gate through `gate_from`.
  While the client is reconnecting, the folders stay as last written.
- Nothing is ever deleted from the mirror directory.

Tests (all through the real reader, in a `tempfile::tempdir()`): write a run, then assert on what
`crate::acpx::bundle::list_runs` / `load_run` (and the `sessions.rs` reader for conversations) return. Cover: a
running run with one finished and one running agent step and activity for the running one (its conversation holds
the joined text and the tool call's latest state); the same run written again with more activity (text grows, no
duplicate trace events, unchanged files untouched); a finished run with no activity (outputs used); less activity
on a later write (nothing lost); a run waiting on a decision gate (`gate_choices` returns the labels with answer
targets); a checkpoint gate; a failed run with its error; a cancelled run; a branching definition's edges; agent
settings read back through `node_settings` / `step_settings`; a running run not stalled after a write and stalled
when its inputs are old and the client is not connected; a summary-only run (no view yet) appearing in the list;
garbage values; `folder_name` / `run_id_of_folder` round trip with a run id containing `/` and spaces;
`is_mirror_dir`.
"""

GATE_PROMPT = f"""
Slice id: gate
Target file: `{PAGER}/piw/gate.rs` (new, in the Grok Build pager library)
Goal: let a person answer a pi-workflows gate (a human decision or a checkpoint a run is parked on) from Grok Build,
in place, so the same run resumes. Today Grok Build can only answer acpx gates, and it does that by starting a new
flow; nothing in it can send an answer to a running workflow server.

What exists. In pi-workflows a run that reaches a `humanDecision` or `checkpoint` node parks on an interactive
request and waits for `decision.answer` / `checkpoint.answer`. Read, as the authority for every field name and
rule below: `src/server/server.ts` (attached whole: `requireSessionCommand`, the two answer handlers, the
coordinator claim in the session watch, `workflowMessage.reportBranch`), `src/server/view.ts` (attached whole: the
session snapshot builder and `currentInteraction`), `src/client/view.ts` (`ClientInteractiveRequest`, the session
snapshot type), `src/workflows/types.ts` (the gate node definitions, `HumanDecisionChoice`, the answer shapes),
`src/workflows/requests.ts` (how a submission is recorded and what it is refused for), and the two attached working
examples of a non-Pi client doing the whole sequence: `test/non-pi-coordinator.test.ts` and
`scripts/live-acpx-routing.mjs`. Facts already checked against that code, which your code must follow:
- The pending question is NOT in the run view. It is the session snapshot's `interaction`
  (`requestId`, `runId`, `attemptId`, `targetSessionId`, `kind`, `contract`, `revision`, `status`). `decision.answer`
  takes the INTERACTION's `revision` as `expectedRevision`.
- Answers are session commands. The server accepts them only on the connection that holds the session's
  coordinator role, which a connection takes with `view.session.watch` carrying `coordinator: true`. The role and
  its epoch belong to the CONNECTION. The epoch is a STRING (`session-epoch-<uuid>`). The snapshot's
  `coordinatorActive` says whether this connection holds the role; `coordinatorEpoch` is the current epoch or null.
- Taking the role is never refused: it REPLACES the current holder and suspends that holder's session. For a run
  that a person started inside Pi, that would cut their live Pi session off from its workflow.
- After taking the role, every session command is refused until the client has reported its branch with
  `workflowMessage.reportBranch` (the snapshot's `branchReportRequired`); both examples do it.
- A decision's choices are a map from key to choice (canonical JSON sorts the keys; author order is not kept). A
  choice may declare a text input; the answer is then the choice plus that input. `decision.answer` is the human
  path: there is no extra field marking an answer as human. `checkpoint.answer` is for checkpoints only, takes any
  JSON value as its input (not approve / reject), and refuses decision gates.

The transport is `PiwClient` (surface below; another engineer is writing it now).
Crates: those of the pager library (serde, serde_json, anyhow, uuid, tracing); dev: tempfile.
{RUST_RULES}
Types you import, do not re-declare:
`use crate::piw::client::{{PiwClient, PiwResponse, RunView, ConnState, CONNECTION_LOST}};`
{PIW_CLIENT_API}
Public surface (implement exactly; the first block is shared with two other files being written now):
{PIW_GATE_API}
```rust
/// What the user chose.
#[derive(Debug, Clone, PartialEq)]
pub enum GateAnswer {{
    /// A decision choice (index into `PiwGate::choices`), with the text for its declared input when it has one.
    Choice {{ index: usize, text: Option<String> }},
    /// A checkpoint's input: any JSON value.
    Input(serde_json::Value),
}}

/// Match what a user typed against a gate.
/// Decision: a 1-based number, a choice key or label (case-insensitive, surrounding space ignored) or an
/// unambiguous prefix of one; anything after the first whitespace following the choice is the text for that
/// choice's input ("2 because the tests fail"). A choice that declares a text input and gets none is an Err that
/// says so; text given to a choice without an input is an Err.
/// Checkpoint: the whole text, as JSON when it parses as JSON, else as a JSON string.
/// Err is one sentence naming the valid choices.
pub fn parse_answer(gate: &PiwGate, typed: &str) -> Result<GateAnswer, String>;

/// The request that answers the gate: (operation, payload, expectedRevision). Pure.
pub fn answer_request(
    gate: &PiwGate,
    answer: &GateAnswer,
    coordinator_epoch: &str,
    submission_id: &str,
) -> Result<(String, serde_json::Value, u64), String>;

/// The session a run belongs to, read from its view.
pub fn session_of_view(view: &RunView) -> Option<String>;

/// The coordinator fields of a session snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorState {{
    pub epoch: Option<String>,        // `coordinatorEpoch`
    pub active: bool,                 // `coordinatorActive`: this connection holds the role
    pub branch_report_required: bool, // `branchReportRequired`
}}
pub fn coordinator_state(session_snapshot: &serde_json::Value) -> CoordinatorState;

#[derive(Debug, Clone, PartialEq)]
pub enum AnswerProgress {{
    /// Still working. Call `poll` again on the next tick.
    Pending,
    /// The server accepted the answer; the run resumes.
    Accepted,
    /// The answer was not delivered. The message is one sentence, ready to show to the user.
    Refused(String),
}}

/// A non-blocking state machine for one answer, driven from the UI tick.
pub struct GateAnswerer {{ /* private */ }}
impl GateAnswerer {{
    pub fn new(client: std::sync::Arc<PiwClient>, gate: PiwGate, answer: GateAnswer) -> Self;
    pub fn poll(&mut self) -> AnswerProgress;
}}

/// The same, blocking, for the helper binary's answer command. Never call on the UI thread.
pub fn answer_blocking(
    client: &PiwClient,
    gate: &PiwGate,
    answer: &GateAnswer,
    timeout: std::time::Duration,
) -> anyhow::Result<()>;
```

Behaviour:
- `gate_from` gives Some only when the snapshot's `interaction` is pending, is of kind "decision" or
  "checkpoint", and belongs to the view's run. The question and the choices come from the interaction's `contract`
  (and, where the contract does not carry them, from the gate node in the view's definition); choices sorted by key.
- The answerer's sequence, each step taken only when the previous one is seen done in the session snapshot or in a
  response, one step per `poll` at most:
  1. Watch the gate's session without claiming (`watch_session(id, false)`) and wait for its snapshot.
  2. Ownership rule. If the snapshot shows this connection already holds the role, go on. If
     `coordinatorEpoch` is null (nobody holds it: a run started outside Pi), claim it:
     `watch_session(id, true)`, and wait for `coordinatorActive`. If another connection holds it, do NOT take
     it: Refused("This run belongs to a live Pi session; answer it there.").
  3. If `branchReportRequired`, send `workflowMessage.reportBranch` exactly as the two examples do and wait for
     its acceptance.
  4. Send the answer built by `answer_request` with `send_keyed`, using one idempotency key for the whole life
     of this answerer, and wait for the response.
- Nothing is sent twice by accident: each step remembers its request id and only acts on that request's response.
- Connection loss (`CONNECTION_LOST` on any step's request, or `PiwClient::connection_count` changed): the role
  and epoch are gone. Go back to step 1 with the same idempotency key for the answer, so an answer the server had
  already applied is not applied again. At most three such restarts, then Refused with a sentence saying the
  connection to the workflow server keeps dropping.
- A refusal of the answer: re-read the session snapshot. If the same request is still pending with a different
  `revision`, rebuild and send again with the new revision (new submission id, new idempotency key), at most
  three times. If the interaction is gone or settled, Refused("This question was already answered."). Otherwise
  Refused with the server's own message in a sentence.
- Waiting has limits: 10 seconds for a snapshot or the role to arrive, 30 seconds for a response; then Refused
  with a sentence naming what did not arrive. Time comes from an injectable clock so tests do not sleep.
- When the answerer finishes (Accepted or Refused) and it was the one that claimed the role, it ends its session
  watch (`unwatch_session`) so the role is not held for the life of the connection.
- `answer_blocking` drives the same machine with short sleeps until it finishes or `timeout` passes.

Tests (pure functions on `serde_json::json!` literals shaped as `src/server/view.ts` builds them; the answerer
cannot be tested against a real `PiwClient`, so write it against a small private trait that `PiwClient`
implements and a test double implements too): `gate_from` for a decision with three choices (sorted by key), a
choice with a text input, a checkpoint, a settled interaction, an interaction of another run, an agent-kind
interaction; `parse_answer` by number, key, label, prefix, ambiguous prefix, out of range, with text for an input,
missing required text, text for a choice without input, checkpoint JSON and plain text; `answer_request` for a
decision, a decision with input and a checkpoint, matching field for field what
`test/non-pi-coordinator.test.ts` and the handlers expect, with the epoch as a string and the interaction revision
returned; `coordinator_state`; `answer_target` / `parse_answer_target` round trip with a run id and key containing
`/`, `%` and spaces, and None for a file path; the answerer: unowned session (claim, report branch, answer,
accepted, watch ended), a session held by another connection (refused, nothing claimed, nothing sent), role
already held, stale revision then success, question already answered, connection lost after the answer was sent
(same idempotency key on the retry), three connection losses, each timeout, no double send across repeated polls.
"""

START_PROMPT = f"""
Slice id: start
Goal: let a program that is not Pi start a pi-workflows run from a workflow file with one command. Grok Build will
run it when the user asks to start a workflow, then watch the run over the server socket. Today only the Pi
extension can start a run: `src/extension/index.ts` (attached whole; see its `run` case) calls
`client.resolveWorkflow(...)` to load the workflow file and compute its `definitionDigest` and `definitionSnapshot`,
builds a run id with `createRunId`, and sends `run.start`. `scripts/live-acpx-routing.mjs` (attached) does the same
by hand for a headless run, and `test/non-pi-coordinator.test.ts` shows a non-Pi client owning a session. The
package's command line is `src/viewer/cli.ts` (attached whole), which has no command to start a run.

Ownership: return exactly these two new files and nothing else:
- `src/viewer/start-command.ts`
- `test/start-command.test.ts`
The coordinator adds the `start` case to `src/viewer/cli.ts` by hand (it will call `runStartCommand`).
{TS_RULES}
`src/viewer` may import `src/state`, `src/workflows`, `src/client`, `src/server`, `src/render`.

Public surface of `src/viewer/start-command.ts`:

```ts
export interface StartCommandIo {{
  stdout: (text: string) => void;
  stderr: (text: string) => void;
}}

/**
 * The part of the workflow client this command uses. Declare it as a `Pick<WorkflowClient, ...>` of exactly the
 * members you call, chosen by reading `src/client/client.ts` and the extension: its `run` case resolves with
 * `resolveWorkflow` and sends through the helper `requestAccepted` (in `src/extension/index.ts`), which calls
 * `ensureAvailable()` and then, because an idempotency key is set, `requestDurable`. Do the same here. Include
 * `endpoint` (the server socket path, public on WorkflowClient) and `close`.
 */
export type StartCommandClient = Pick<WorkflowClient, /* the members used */>;

export interface StartCommandDeps {{
  /**
   * Gives a client for the project at `cwd` with the server running (the other CLI commands call
   * `ensureRunning()` first; see `src/viewer/cli.ts`). Injected so tests use a fake.
   */
  connect: (options: {{ cwd: string }}) => Promise<StartCommandClient>;
  /** Reads `--input-file`. Default: node:fs/promises readFile as utf8. */
  readFile?: (path: string) => Promise<string>;
  now?: () => Date;
}}

/**
 * `pi-workflows start <workflow file or name> [--input <json> | --input-file <path>] [--cwd <dir>]
 *                     [--session <id>] [--json]`
 * Resolves the workflow, starts it headless, prints the run id. Returns the process exit code.
 */
export async function runStartCommand(
  argv: string[],
  io: StartCommandIo,
  deps: StartCommandDeps,
): Promise<number>;

/** Parsed arguments; exported for tests. Pure (reads no file). Throws Error with a usage message on bad arguments. */
export function parseStartArgs(argv: string[]): {{
  workflowRef: string;
  /** From `--input`; absent when `--input-file` is used or neither is given. */
  input?: JsonValue;
  inputFile?: string;
  cwd: string;
  sessionId?: string;
  json: boolean;
}};
```
`createRunId` is exported from `src/workflows/store.ts` (not attached: it is large) as
`export function createRunId(workflowName: string, now: Date = new Date()): string`; import it the way
`src/extension/index.ts` does.

Behaviour:
- Builds the `run.start` request exactly as the extension's `run` case and `scripts/live-acpx-routing.mjs` do
  (same payload fields, `executionMode: "headless"`, run id from the same `createRunId`), with `originSessionId`
  = `--session` when given, else a fresh id of the form the server accepts for a non-Pi session (see the test and
  the script). Steps that name an acpx agent run through acpx; the others run in a headless Pi child; this command
  does not choose.
- `--input` is a JSON value; `--input-file` names a file holding one; with neither the input is `{{}}`.
- Output with `--json`: one line of canonical JSON
  `{{"runId":…,"sessionId":…,"workflowName":…,"socketPath":…,"revision":…}}` and nothing else on stdout.
  Without `--json`: one human line with the run id and the workflow name.
- Exit codes. 2, with the usage on stderr, for anything wrong with the arguments themselves: a missing workflow,
  an unknown flag, a flag without its value, both `--input` and `--input-file`, `--input` text that is not JSON
  (all thrown by `parseStartArgs`). 1, with one clear message on stderr that includes the loader's or server's own
  message, for everything that fails afterwards: an `--input-file` that cannot be read or does not hold JSON, a
  workflow that fails to load or validate, a server that cannot be reached, a server refusal. 0 on success. The
  client is closed on every path after it was connected.
- Nothing here waits for the run to finish.

Tests (`test/start-command.test.ts`): argument parsing (each flag, both input forms, both given, missing workflow,
unknown flag, invalid `--input` JSON, exit code 2 through `runStartCommand`); an unreadable and a non-JSON
`--input-file` (exit code 1); a successful start through a fake client asserting the exact `run.start` request
(operation, runId on the envelope, every payload field) and both output forms; `--session` passed through; a
resolve failure; a server refusal; the client closed after success and after failure.
{MULTI_FILE_RETURN}
"""

TASKS: List[TaskSpec] = [
    {
        "custom_id": "activity",
        "output_files": ["piw:src/state/step-activity.ts", "piw:src/server/step-activity.ts", "piw:test/step-activity.test.ts"],
        "context_files": [
            "piw:AGENTS.md",
            "piw:slophammer.yml",
            "piw:src/state/json.ts",
            "piw:src/client/protocol.ts",
            "piw:src/client/view.ts",
            "piw:src/workflows/types.ts",
            "piw:src/server/workflow-runner-protocol.ts",
            "piw:src/server/workflow-runner-entry.ts",
            "piw:src/server/server.ts",
            "piw:src/server/acpx-executor.ts",
            "piw:src/server/acpx-submission-channel.ts",
            "piw:src/server/rpc-executor.ts",
            "piw:src/server/agent-routing-executor.ts",
            "piw:test/acpx-executor.test.ts",
            "piw:test/agent-routing.test.ts",
            "piw:node_modules/acpx/dist/runtime.d.ts",
            f"piw:{PI_DOCS}/rpc.md",
            f"piw:{PI_DOCS}/json.md",
        ],
        "prompt": ACTIVITY_PROMPT,
    },
]

PIW_TUI = ["piw:tui/src/client.rs", "piw:tui/src/protocol.rs", "piw:tui/src/client_commands.rs", "piw:tui/src/state/types.rs", "piw:tui/src/main.rs"]
PIW_VIEW = ["piw:src/client/protocol.ts", "piw:src/client/view.ts", "piw:src/server/view.ts", "piw:src/workflows/types.ts", "piw:src/server/server.ts"]
TASKS += [
    {
        "custom_id": "start",
        "output_files": ["piw:src/viewer/start-command.ts", "piw:test/start-command.test.ts"],
        "context_files": ["piw:AGENTS.md", "piw:slophammer.yml", "piw:src/state/json.ts", "piw:src/client/protocol.ts", "piw:src/client/client.ts", "piw:src/viewer/cli.ts", "piw:src/extension/index.ts", "piw:src/workflows/loader.ts", "piw:src/server/server.ts", "piw:scripts/live-acpx-routing.mjs", "piw:test/non-pi-coordinator.test.ts", "piw:test/agent-routing.test.ts"],
        "prompt": START_PROMPT,
    },
    {
        "custom_id": "client",
        "output_files": [f"grok:{PAGER}/piw/client.rs"],
        "context_files": PIW_TUI + ["piw:src/state/json.ts", "piw:src/state/database.ts", "piw:src/client/protocol.ts", "piw:src/client/client.ts", "piw:src/client/view.ts", "piw:src/server/view.ts", "piw:src/server/server.ts", "piw:test/non-pi-coordinator.test.ts", "grok:crates/codegen/pi-grok-adapter/src/subagent_transport.rs", f"grok:{PAGER}/acpx/graph.rs"],
        "prompt": CLIENT_PROMPT,
    },
    {
        "custom_id": "driver",
        "output_files": [f"grok:{BIN}/piw_driver.rs"],
        "context_files": [f"grok:{BIN}/driver.rs", f"grok:{BIN}/project.rs", f"grok:{BIN}/agent.rs", f"grok:{BIN}/main.rs"] + PIW_VIEW + ["piw:tui/src/state/types.rs", "piw:tui/src/client.rs"],
        "prompt": DRIVER_PROMPT,
    },
    {
        "custom_id": "mirror",
        "output_files": [f"grok:{PAGER}/piw/mirror.rs"],
        "context_files": [f"grok:{PAGER}/acpx/bundle.rs", f"grok:{PAGER}/acpx/sessions.rs", f"grok:{PAGER}/acpx/graph.rs", f"grok:{PAGER}/acpx/mod.rs", f"grok:{PAGER}/views/acpx_modal/run.rs", f"grok:{PAGER}/views/acpx_modal/detail.rs", f"grok:{PAGER}/views/acpx_modal/ledger.rs", f"grok:{PAGER}/views/side_panel/flows.rs", f"grok:{PAGER}/app/dispatch/acpx_run.rs", "grok:task-scripts/acpx-mode.py", "piw:src/client/view.ts", "piw:src/server/view.ts", "piw:src/workflows/types.ts", "piw:tui/src/state/types.rs", "piw:tui/src/client.rs"],
        "prompt": MIRROR_PROMPT,
    },
    {
        "custom_id": "gate",
        "output_files": [f"grok:{PAGER}/piw/gate.rs"],
        "context_files": PIW_VIEW + ["piw:src/state/json.ts", "piw:src/workflows/requests.ts", "piw:src/client/client.ts", "piw:test/non-pi-coordinator.test.ts", "piw:scripts/live-acpx-routing.mjs", "piw:tui/src/state/types.rs", f"grok:{PAGER}/acpx/bundle.rs", f"grok:{BIN}/agent.rs"],
        "prompt": GATE_PROMPT,
    },
]

REPOS = {"piw": PIW, "grok": GROK}


def locate(entry: str):
    """'piw:src/x.ts' -> (repo label, repo root, relative path)."""
    label, _, rel = entry.partition(":")
    if label not in REPOS or not rel:
        sys.exit(f"bad file entry {entry!r}: expected piw:<path> or grok:<path>")
    return label, REPOS[label].resolve(), rel


FILE_RE = re.compile(r"^=== FILE: (?P<path>[^\n]+?) ===[ \t]*\n```[a-zA-Z0-9_-]*[ \t]*\n(?P<body>[\s\S]*?)\n```[ \t]*(?=\n+=== FILE: |\s*\Z)", re.M)


def split_files(content: str) -> Dict[str, str]:
    content = re.sub(r"^<think>[\s\S]*?</think>\s*", "", (content or "").replace("\r\n", "\n"))
    return {m.group("path").strip().strip("`"): m.group("body") for m in FILE_RE.finditer(content)}


def build_prompt(task: TaskSpec) -> str:
    blocks = []
    for entry in task.get("context_files", []):
        label, repo, rel = locate(entry)
        path = (repo / rel).resolve()
        if not path.is_relative_to(repo):
            sys.exit(f"{task['custom_id']}: context file outside its repo: {entry}")
        if not path.is_file():
            sys.exit(f"{task['custom_id']}: context file not found: {path}")
        blocks.append(f"### `{rel}` (complete file, repository {repo.name})\n````\n{path.read_text(encoding='utf-8', errors='replace')}\n````")
    return (
        f"=== REPOSITORY CONTEXT ({len(blocks)} complete files) ===\n\n" + "\n\n".join(blocks)
        + "\n\n=== TASK SPECIFICATION ===\n\n" + task["prompt"].strip() + "\n"
    )


def post(key: str, payload: dict) -> dict:
    body = json.dumps(payload).encode("utf-8")
    last = "no attempt made"
    for attempt in range(MAX_RETRIES):
        req = urllib.request.Request(ENDPOINT, data=body, method="POST", headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=TIMEOUT) as res:
                data = json.loads(res.read().decode("utf-8"))
            if "error" not in data:
                return data
            last = str(data["error"].get("message", data["error"]))[:300]
        except urllib.error.HTTPError as e:
            last = f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:300]}"
            if e.code in (400, 401, 402, 403, 404):
                break
        except Exception as e:  # timeouts, resets
            last = f"{type(e).__name__}: {e}"
        time.sleep(RETRY_DELAY * (attempt + 1))
    raise RuntimeError(last)


def run_task(key: str, model: str, task: TaskSpec, out: Path, effort: str = "") -> Dict[str, Any]:
    start = time.time()
    payload = {"model": model, "temperature": 0.0, "max_tokens": MAX_TOKENS, "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": build_prompt(task)}]}
    if effort:  # thinking counts against max_tokens: at the default level it used 31-46k of 64k (2026-11-21)
        payload["reasoning"] = {"effort": effort}
    try:
        res = post(key, payload)
        choice = (res.get("choices") or [{}])[0]
        raw = (choice.get("message") or {}).get("content") or ""
        returns = out / "_returns" / f"{task['custom_id']}.md"
        returns.parent.mkdir(parents=True, exist_ok=True)
        returns.write_text(raw, encoding="utf-8")
        usage = res.get("usage") or {}
        (out / "_returns" / f"{task['custom_id']}.usage.json").write_text(json.dumps({"usage": usage, "model": res.get("model"), "seconds": round(time.time() - start, 1), "finish": choice.get("finish_reason")}, indent=2), encoding="utf-8")
        if str(choice.get("finish_reason") or choice.get("native_finish_reason")).lower() in ("length", "max_tokens"):
            return {"task": task, "ok": False, "error": f"cut off at max_tokens={MAX_TOKENS}; partial return kept in {returns}"}
        files = split_files(raw)
        wanted = {locate(e)[2]: locate(e)[1].name for e in task["output_files"]}
        missing = [f for f in wanted if f not in files]
        extra = [f for f in files if f not in wanted]
        for rel, repo_name in wanted.items():
            if rel in files:
                target = out / repo_name / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(files[rel] + "\n", encoding="utf-8")
        if missing:
            return {"task": task, "ok": False, "error": f"missing files {missing} (extra: {extra}); raw return in {returns}"}
        return {"task": task, "ok": True, "seconds": time.time() - start, "usage": usage, "lines": {f: files[f].count(chr(10)) + 1 for f in wanted}, "extra": extra}
    except Exception as e:
        return {"task": task, "ok": False, "error": str(e)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--model", default=MODEL)
    ap.add_argument("--output", default=str(HERE / "piw-in-grok_out"))
    ap.add_argument("--only", help="comma-separated slice ids")
    ap.add_argument("--effort", default="", help="thinking level sent as reasoning.effort (low|medium|high); default: the model's own")
    ap.add_argument("--dry-run", action="store_true", help="write full prompts to <output>/_prompts, print sizes and a cost estimate, call nothing")
    args = ap.parse_args()
    out = Path(args.output).resolve()
    tasks = [t for t in TASKS if not args.only or t["custom_id"] in args.only.split(",")]
    if not tasks:
        sys.exit(f"--only matched nothing; ids: {', '.join(t['custom_id'] for t in TASKS)}")
    if args.dry_run:
        total = 0
        for t in tasks:
            prompt = build_prompt(t)
            total += len(prompt)
            p = out / "_prompts" / f"{t['custom_id']}.md"
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(prompt, encoding="utf-8")
            print(f"{t['custom_id']}: {len(prompt):,} chars (~{len(prompt) // 4:,} tokens) in, task text {len(t['prompt']):,} chars, {len(t['context_files'])} whole context files")
            for entry in t["context_files"]:
                print(f"    {(locate(entry)[1] / locate(entry)[2]).stat().st_size:>8,}  {entry}")
            print(f"    returns: {', '.join(t['output_files'])}\n    prompt written to {p}")
        est = total / 4 * PRICE_IN_PER_M / 1e6 + len(tasks) * 20000 * PRICE_OUT_PER_M / 1e6
        print(f"\n[dry run] {len(tasks)} slice(s) on {args.model}: ~{total // 4:,} tokens in, about ${est:.2f} assuming 20k tokens out per slice. Nothing was sent.")
        return
    key = os.environ.get("OPENROUTER_API_KEY")
    if not key:
        sys.exit("OPENROUTER_API_KEY is not set")
    print(f"Sending {len(tasks)} slice(s) to {args.model}; output in {out}", flush=True)
    failed = False
    import threading
    started = time.time()
    pending = {t["custom_id"] for t in tasks}
    done = threading.Event()

    def ticker():  # the batch must not be a black box while requests are out
        while not done.wait(20):
            print(f"  ...  {time.time() - started:4.0f}s  waiting on: {', '.join(sorted(pending))}", flush=True)

    threading.Thread(target=ticker, daemon=True).start()
    with ThreadPoolExecutor(max_workers=min(len(tasks), 16)) as pool:
        futures = []
        for t in tasks:
            print(f"  sent {t['custom_id']}: {len(build_prompt(t)) // 4:,} tokens in", flush=True)
            futures.append(pool.submit(run_task, key, args.model, t, out, args.effort))
        for fut in as_completed(futures):
            r = fut.result()
            tid = r["task"]["custom_id"]
            pending.discard(tid)
            if r["ok"]:
                print(f"  ok   {tid}: {r['seconds']:.0f}s, lines {r['lines']}, usage {r['usage'].get('prompt_tokens')}/{r['usage'].get('completion_tokens')}" + (f", extra files ignored: {r['extra']}" if r["extra"] else ""), flush=True)
            else:
                print(f"  FAIL {tid}: {r['error']}", flush=True)
                failed = True
    done.set()
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
