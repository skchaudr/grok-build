=== FILE: src/state/step-activity.ts ===
```ts
import { canonicalJson, parseJson, type JsonValue } from "./json.js";

export const STEP_ACTIVITY_SCHEMA = "pi-workflows.step-activity.v1" as const;
export const STEP_ACTIVITY_KINDS = ["text", "thought", "tool", "status"] as const;
export type StepActivityKind = (typeof STEP_ACTIVITY_KINDS)[number];

export const STEP_ACTIVITY_TOOL_STATUSES = [
  "pending",
  "in_progress",
  "completed",
  "failed",
] as const;
export type StepActivityToolStatus = (typeof STEP_ACTIVITY_TOOL_STATUSES)[number];

/** State of one tool call. Sent again, whole, each time the call changes. */
export interface StepActivityTool {
  toolCallId: string;
  /** Short name of the tool or of the call, for a one-line row. */
  title: string;
  /** "pending" | "in_progress" | "completed" | "failed" when the source says; absent when it does not. */
  status?: StepActivityToolStatus;
  /** The source's own category word (acpx `kind`, e.g. "read", "edit", "execute"; Pi: the tool name). */
  kind?: string;
  input?: JsonValue;
  output?: JsonValue;
  /** True when `input` or `output` was shortened to fit a protocol frame (see the batcher). */
  truncated?: boolean;
}

/** What an executor reports while a step runs. Never persisted. */
export interface StepActivityBody {
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
}

export interface StepActivityKey {
  runId: string;
  nodeId: string;
  attemptId: string;
}

/** One numbered activity record as clients receive it. */
export interface StepActivity extends StepActivityBody, StepActivityKey {
  schema: typeof STEP_ACTIVITY_SCHEMA;
  /** Per run, starts at 1, increases by exactly 1 with every record of the run, across all its attempts. */
  seq: number;
  /** ISO-8601 UTC time the server numbered the record. */
  at: string;
}

const ACTIVITY_REQUIRED_KEYS = [
  "schema",
  "runId",
  "nodeId",
  "attemptId",
  "seq",
  "at",
  "kind",
  "text",
] as const;
const ACTIVITY_OPTIONAL_KEYS = ["tool"] as const;
const BODY_REQUIRED_KEYS = ["kind", "text"] as const;
const BODY_OPTIONAL_KEYS = ["tool"] as const;
const TOOL_REQUIRED_KEYS = ["toolCallId", "title"] as const;
const TOOL_OPTIONAL_KEYS = ["status", "kind", "input", "output", "truncated"] as const;

/** Strict: exact keys, known kind, `tool` present iff kind is "tool", seq a positive safe integer. Throws Error. */
export function parseStepActivity(value: unknown): StepActivity {
  const record = requireRecord(value, "stepActivity");
  requireKeys(record, ACTIVITY_REQUIRED_KEYS, ACTIVITY_OPTIONAL_KEYS, "stepActivity");
  if (record.schema !== STEP_ACTIVITY_SCHEMA) {
    throw new Error(`stepActivity.schema must be ${STEP_ACTIVITY_SCHEMA}`);
  }
  const body = parseBodyFields(record);
  return {
    schema: STEP_ACTIVITY_SCHEMA,
    runId: requireNonEmptyString(record.runId, "runId"),
    nodeId: requireNonEmptyString(record.nodeId, "nodeId"),
    attemptId: requireNonEmptyString(record.attemptId, "attemptId"),
    seq: requirePositiveInteger(record.seq, "seq"),
    at: requireNonEmptyString(record.at, "at"),
    ...body,
  };
}

/** Strict in the same way. Throws Error. */
export function parseStepActivityBody(value: unknown): StepActivityBody {
  const record = requireRecord(value, "stepActivityBody");
  requireKeys(record, BODY_REQUIRED_KEYS, BODY_OPTIONAL_KEYS, "stepActivityBody");
  return parseBodyFields(record);
}

/** Array of bodies, as the runner sends a batch. Throws Error on a non-array or any bad element. */
export function parseStepActivityBodies(value: unknown): StepActivityBody[] {
  if (!Array.isArray(value)) {
    throw new Error("stepActivityBodies must be an array");
  }
  return value.map((item) => parseStepActivityBody(item));
}

function parseBodyFields(record: Record<string, unknown>): StepActivityBody {
  const kind = requireKind(record.kind);
  const text = requireString(record.text, "text");
  const hasTool = Object.hasOwn(record, "tool");
  if (kind === "tool" && !hasTool) {
    throw new Error('tool is required when kind is "tool"');
  }
  if (kind !== "tool" && hasTool) {
    throw new Error(`tool is not allowed when kind is "${kind}"`);
  }
  return hasTool ? { kind, text, tool: parseStepActivityTool(record.tool) } : { kind, text };
}

function parseStepActivityTool(value: unknown): StepActivityTool {
  const record = requireRecord(value, "tool");
  requireKeys(record, TOOL_REQUIRED_KEYS, TOOL_OPTIONAL_KEYS, "tool");
  const tool: StepActivityTool = {
    toolCallId: requireNonEmptyString(record.toolCallId, "tool.toolCallId"),
    title: requireString(record.title, "tool.title"),
  };
  if (Object.hasOwn(record, "status")) {
    tool.status = requireToolStatus(record.status);
  }
  if (Object.hasOwn(record, "kind")) {
    tool.kind = requireString(record.kind, "tool.kind");
  }
  if (Object.hasOwn(record, "input")) {
    tool.input = requireJsonValue(record.input, "tool.input");
  }
  if (Object.hasOwn(record, "output")) {
    tool.output = requireJsonValue(record.output, "tool.output");
  }
  if (Object.hasOwn(record, "truncated")) {
    tool.truncated = requireBoolean(record.truncated, "tool.truncated");
  }
  return tool;
}

function requireRecord(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requireKeys(
  record: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[],
  label: string,
): void {
  const allowed = new Set([...required, ...optional]);
  for (const key of Object.keys(record)) {
    if (!allowed.has(key)) {
      throw new Error(`Unknown ${label} field: ${key}`);
    }
  }
  for (const key of required) {
    if (!Object.hasOwn(record, key)) {
      throw new Error(`Missing required ${label} field: ${key}`);
    }
  }
}

function requireNonEmptyString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return value;
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new Error(`${field} must be a string`);
  }
  return value;
}

function requireBoolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw new Error(`${field} must be a boolean`);
  }
  return value;
}

function requirePositiveInteger(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${field} must be a positive safe integer`);
  }
  return value;
}

function requireKind(value: unknown): StepActivityKind {
  if (typeof value !== "string" || !STEP_ACTIVITY_KINDS.includes(value as StepActivityKind)) {
    throw new Error("kind must be one of text, thought, tool, status");
  }
  return value as StepActivityKind;
}

function requireToolStatus(value: unknown): StepActivityToolStatus {
  if (
    typeof value !== "string" ||
    !STEP_ACTIVITY_TOOL_STATUSES.includes(value as StepActivityToolStatus)
  ) {
    throw new Error("tool.status must be one of pending, in_progress, completed, failed");
  }
  return value as StepActivityToolStatus;
}

function requireJsonValue(value: unknown, field: string): JsonValue {
  try {
    return parseJson(canonicalJson(value));
  } catch {
    throw new Error(`${field} must be a valid JSON value`);
  }
}
```

=== FILE: src/server/step-activity.ts ===
```ts
import type { AcpRuntimeEvent } from "acpx/runtime";
import { canonicalJson, parseJson, type JsonValue } from "../state/json.js";
import {
  STEP_ACTIVITY_SCHEMA,
  STEP_ACTIVITY_TOOL_STATUSES,
  type StepActivity,
  type StepActivityBody,
  type StepActivityKey,
  type StepActivityTool,
  type StepActivityToolStatus,
} from "../state/step-activity.js";

const DEFAULT_FLUSH_MS = 100;
/**
 * Default batch cap: a quarter of the 1 MiB workflow runner and client protocol frame limit
 * (`MAX_WORKFLOW_RUNNER_PROTOCOL_MESSAGE_BYTES` / `MAX_PROTOCOL_MESSAGE_BYTES`), leaving room for
 * the surrounding protocol envelope.
 */
const DEFAULT_MAX_BATCH_BYTES = 256 * 1024;
const MIN_BATCH_BYTES = 1024;

const IGNORED_ACPX_STATUS_TAGS = new Set<string>([
  "usage_update",
  "available_commands_update",
  "plan",
]);
const IGNORED_ACPX_STATUS_FIELDS = [
  "used",
  "size",
  "cost",
  "breakdown",
  "availableCommands",
  "entries",
] as const;

/** acpx event of a running turn -> activity, or undefined for events that carry nothing to show. */
export function stepActivityFromAcpx(event: AcpRuntimeEvent): StepActivityBody | undefined {
  switch (event.type) {
    case "text_delta":
      return acpxTextDelta(event);
    case "status":
      return acpxStatus(event);
    case "tool_call":
      return acpxToolCall(event);
    default:
      return undefined;
  }
}

/** One parsed record of Pi's RPC stdout stream -> activity, or undefined. Never throws, whatever the input. */
export function stepActivityFromPiRpc(record: unknown): StepActivityBody | undefined {
  try {
    return mapPiRpcRecord(record);
  } catch {
    return undefined;
  }
}

/** Splits a byte stream into JSON records on LF only (see Pi's rpc.md "Framing"), tolerating CR before LF. */
export class JsonlRecordReader {
  private buffered: Buffer = Buffer.alloc(0);

  /** Feed a chunk; returns the complete records it finished. A line that is not valid JSON is skipped. */
  push(chunk: Buffer | string): unknown[] {
    const incoming = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk, "utf8");
    if (incoming.byteLength === 0) return [];
    this.buffered =
      this.buffered.byteLength === 0 ? incoming : Buffer.concat([this.buffered, incoming]);
    const records: unknown[] = [];
    for (;;) {
      const newline = this.buffered.indexOf(0x0a);
      if (newline < 0) break;
      const line = stripTrailingCr(this.buffered.subarray(0, newline));
      this.buffered = this.buffered.subarray(newline + 1);
      const parsed = tryParseJsonLine(line);
      if (parsed.ok) records.push(parsed.value);
    }
    return records;
  }

  /** The stream ended: returns the final unterminated line as a record if it is valid JSON, else []. */
  end(): unknown[] {
    if (this.buffered.byteLength === 0) return [];
    const remaining = stripTrailingCr(this.buffered);
    this.buffered = Buffer.alloc(0);
    const parsed = tryParseJsonLine(remaining);
    return parsed.ok ? [parsed.value] : [];
  }
}

export interface StepActivityBatcherOptions {
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
}

/** Runner side. Coalesces a token-by-token stream into few, frame-safe batches without losing or reordering text. */
export class StepActivityBatcher {
  private readonly emit: (batch: StepActivityBody[]) => void;
  private readonly flushMs: number;
  private readonly maxBatchBytes: number;
  private readonly setTimer: (run: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;
  private waiting: StepActivityBody[] = [];
  private timer: unknown = undefined;
  private closed = false;

  constructor(options: StepActivityBatcherOptions) {
    this.emit = options.emit;
    this.flushMs = options.flushMs ?? DEFAULT_FLUSH_MS;
    this.maxBatchBytes = normalizeMaxBatchBytes(options.maxBatchBytes);
    this.setTimer = options.setTimer ?? defaultSetTimer;
    this.clearTimer = options.clearTimer ?? defaultClearTimer;
  }

  push(body: StepActivityBody): void {
    if (this.closed) return;
    if (body.kind === "text" || body.kind === "thought") {
      this.pushTextLike(body.kind, body.text);
      return;
    }
    if (body.kind === "status") {
      this.pushStatus(body.text);
      return;
    }
    this.pushTool(body);
  }

  /** Emit whatever is waiting now. */
  flush(): void {
    this.disarmTimer();
    if (this.waiting.length === 0) return;
    const batch = this.waiting;
    this.waiting = [];
    this.emitBatch(batch);
  }

  /** flush(), then ignore every later push. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.flush();
  }

  private pushTextLike(kind: "text" | "thought", text: string): void {
    if (this.tryMergeTextLike(kind, text)) return;
    if (text.length === 0) {
      this.appendOrFlush({ kind, text: "" });
      return;
    }
    this.enqueueTextChunks(kind, text);
  }

  private tryMergeTextLike(kind: "text" | "thought", text: string): boolean {
    const last = this.waiting[this.waiting.length - 1];
    if (last === undefined || last.kind !== kind) return false;
    if (last.text.length + text.length > this.maxBatchBytes) return false;
    const merged: StepActivityBody = { kind, text: last.text + text };
    const candidate = [...this.waiting.slice(0, -1), merged];
    if (canonicalBytes(candidate) > this.maxBatchBytes) return false;
    this.waiting[this.waiting.length - 1] = merged;
    return true;
  }

  private enqueueTextChunks(kind: "text" | "thought", text: string): void {
    if (text.length <= this.maxBatchBytes) {
      const body: StepActivityBody = { kind, text };
      if (canonicalBytes([...this.waiting, body]) <= this.maxBatchBytes) {
        this.waiting.push(body);
        this.armTimerIfNeeded();
        return;
      }
    }
    this.flush();
    const fits = (slice: string) => canonicalBytes([{ kind, text: slice }]) <= this.maxBatchBytes;
    let offset = 0;
    while (offset < text.length) {
      const len = maxFittingCodePointLength(text, offset, fits, this.maxBatchBytes);
      if (len <= 0) break;
      const chunk: StepActivityBody = { kind, text: text.slice(offset, offset + len) };
      offset += len;
      if (offset < text.length) {
        this.emitBatch([chunk]);
      } else {
        this.waiting.push(chunk);
        this.armTimerIfNeeded();
      }
    }
  }

  private pushStatus(text: string): void {
    const fitted = fitStatusBody(text, this.maxBatchBytes);
    const last = this.waiting[this.waiting.length - 1];
    if (last?.kind === "status") {
      this.waiting.pop();
    }
    this.appendOrFlush(fitted);
  }

  private pushTool(body: StepActivityBody): void {
    const fitted = fitToolBody(body, this.maxBatchBytes);
    const last = this.waiting[this.waiting.length - 1];
    if (sameToolCall(last, fitted)) {
      this.waiting.pop();
    }
    this.appendOrFlush(fitted);
  }

  private appendOrFlush(body: StepActivityBody): void {
    if (this.waiting.length > 0 && canonicalBytes([...this.waiting, body]) > this.maxBatchBytes) {
      this.flush();
    }
    this.waiting.push(body);
    this.armTimerIfNeeded();
  }

  private armTimerIfNeeded(): void {
    if (this.timer !== undefined || this.waiting.length === 0) return;
    this.timer = this.setTimer(() => {
      this.timer = undefined;
      this.flush();
    }, this.flushMs);
  }

  private disarmTimer(): void {
    if (this.timer === undefined) return;
    const handle = this.timer;
    this.timer = undefined;
    this.clearTimer(handle);
  }

  private emitBatch(batch: StepActivityBody[]): void {
    if (batch.length === 0) return;
    try {
      this.emit(batch);
    } catch {
      // Transport is unavailable; drop only this batch and keep accepting future activity.
    }
  }
}

export type StepActivityListener = (activities: StepActivity[]) => void;

type HubSubscription = {
  listener: StepActivityListener;
};

type RunActivityState = {
  records: StepActivity[];
  subscriptions: Set<HubSubscription>;
};

/** Server side. One per server. Numbers, remembers and fans out activity. Holds no durable state. */
export class StepActivityHub {
  private readonly now: () => Date;
  private readonly runs = new Map<string, RunActivityState>();

  constructor(options?: { now?: () => Date }) {
    this.now = options?.now ?? (() => new Date());
  }

  /** Number the bodies (seq, at, schema, key), remember them, hand them to the run's listeners, return them. */
  publish(key: StepActivityKey, bodies: StepActivityBody[]): StepActivity[] {
    if (bodies.length === 0) return [];
    const entry = this.ensureRun(key.runId);
    const at = this.now().toISOString();
    const created: StepActivity[] = [];
    for (const body of bodies) {
      const seq = entry.records.length + 1;
      const activity: StepActivity = {
        schema: STEP_ACTIVITY_SCHEMA,
        runId: key.runId,
        nodeId: key.nodeId,
        attemptId: key.attemptId,
        seq,
        at,
        kind: body.kind,
        text: body.text,
        ...(body.tool === undefined ? {} : { tool: { ...body.tool } }),
      };
      entry.records.push(activity);
      created.push(activity);
    }
    for (const subscription of [...entry.subscriptions]) {
      if (this.runs.get(key.runId) !== entry) break;
      if (!entry.subscriptions.has(subscription)) continue;
      this.deliver(entry, subscription, created);
    }
    return created;
  }

  /** Everything remembered for the run with seq > afterSeq, in seq order. Unknown run: []. */
  read(runId: string, afterSeq: number): StepActivity[] {
    const entry = this.runs.get(runId);
    if (entry === undefined) return [];
    const start = afterSeq <= 0 ? 0 : Math.floor(afterSeq);
    if (start >= entry.records.length) return [];
    return entry.records.slice(start);
  }

  /**
   * Calls `listener` synchronously, once, with read(runId, afterSeq) if that is non-empty, then with every later
   * publish of the run, until the returned function is called. Calling it twice is harmless.
   */
  subscribe(runId: string, afterSeq: number, listener: StepActivityListener): () => void {
    const entry = this.ensureRun(runId);
    const subscription: HubSubscription = { listener };
    entry.subscriptions.add(subscription);
    const backlog = this.read(runId, afterSeq);
    if (backlog.length > 0) {
      this.deliver(entry, subscription, backlog);
    }
    return () => {
      entry.subscriptions.delete(subscription);
      this.pruneEmptyRun(runId, entry);
    };
  }

  /** The seq of the newest record of the run, 0 if none. */
  lastSeq(runId: string): number {
    return this.runs.get(runId)?.records.length ?? 0;
  }

  /** The run is over or was removed: forget its records and its listeners. Later publishes start again at seq 1. */
  closeRun(runId: string): void {
    this.runs.delete(runId);
  }

  /**
   * Where a watch asking for `afterSeq` really starts. When `afterSeq` is greater than lastSeq(runId) the hub's
   * numbering does not continue the client's (the server restarted): `{ afterSeq: 0, reset: true }`, and the
   * client is told to discard what it holds. Otherwise `{ afterSeq, reset: false }`.
   */
  resumePoint(runId: string, afterSeq: number): { afterSeq: number; reset: boolean } {
    if (afterSeq > this.lastSeq(runId)) {
      return { afterSeq: 0, reset: true };
    }
    return { afterSeq, reset: false };
  }

  private ensureRun(runId: string): RunActivityState {
    const existing = this.runs.get(runId);
    if (existing !== undefined) return existing;
    const created: RunActivityState = { records: [], subscriptions: new Set() };
    this.runs.set(runId, created);
    return created;
  }

  private deliver(
    entry: RunActivityState,
    subscription: HubSubscription,
    activities: StepActivity[],
  ): void {
    try {
      subscription.listener([...activities]);
    } catch {
      entry.subscriptions.delete(subscription);
    }
  }

  private pruneEmptyRun(runId: string, entry: RunActivityState): void {
    if (
      this.runs.get(runId) === entry &&
      entry.records.length === 0 &&
      entry.subscriptions.size === 0
    ) {
      this.runs.delete(runId);
    }
  }
}

/**
 * Split records into groups so each group's canonical JSON is at most maxBytes, keeping order, never splitting a
 * record, never returning an empty group. A single record larger than maxBytes gets a group of its own.
 */
export function stepActivityFrames(activities: StepActivity[], maxBytes: number): StepActivity[][] {
  const frames: StepActivity[][] = [];
  let current: StepActivity[] = [];
  let currentBytes = 0;
  for (const activity of activities) {
    const recordBytes = canonicalBytes(activity);
    if (current.length === 0) {
      current.push(activity);
      currentBytes = 2 + recordBytes;
    } else if (currentBytes + 1 + recordBytes <= maxBytes) {
      current.push(activity);
      currentBytes += 1 + recordBytes;
    } else {
      frames.push(current);
      current = [activity];
      currentBytes = 2 + recordBytes;
    }
  }
  if (current.length > 0) frames.push(current);
  return frames;
}

function acpxTextDelta(
  event: Extract<AcpRuntimeEvent, { type: "text_delta" }>,
): StepActivityBody | undefined {
  if (event.text.length === 0) return undefined;
  return {
    kind: event.stream === "thought" ? "thought" : "text",
    text: event.text,
  };
}

function acpxStatus(
  event: Extract<AcpRuntimeEvent, { type: "status" }>,
): StepActivityBody | undefined {
  if (event.text.length === 0 || isStructuredAcpxStatus(event)) return undefined;
  return { kind: "status", text: event.text };
}

function isStructuredAcpxStatus(event: Extract<AcpRuntimeEvent, { type: "status" }>): boolean {
  if (event.tag !== undefined && IGNORED_ACPX_STATUS_TAGS.has(event.tag)) return true;
  return IGNORED_ACPX_STATUS_FIELDS.some((field) => event[field] !== undefined);
}

function acpxToolCall(
  event: Extract<AcpRuntimeEvent, { type: "tool_call" }>,
): StepActivityBody {
  const tool: StepActivityTool = {
    toolCallId: nonEmptyString(event.toolCallId) ?? nonEmptyString(event.title) ?? "tool",
    title: nonEmptyString(event.title) ?? nonEmptyString(event.text) ?? "tool",
  };
  if (isToolStatus(event.status)) tool.status = event.status;
  if (typeof event.kind === "string") tool.kind = event.kind;
  assignOptionalJson(tool, "input", event.rawInput);
  assignOptionalJson(tool, "output", event.rawOutput);
  return { kind: "tool", text: event.text, tool };
}

function mapPiRpcRecord(record: unknown): StepActivityBody | undefined {
  if (!isRecord(record) || typeof record.type !== "string") return undefined;
  switch (record.type) {
    case "message_update":
      return piMessageUpdate(record);
    case "tool_execution_start":
      return piToolExecution(record, "in_progress", true, undefined);
    case "tool_execution_update":
      return piToolExecution(record, "in_progress", true, "partialResult");
    case "tool_execution_end":
      return piToolExecution(
        record,
        record.isError === true ? "failed" : "completed",
        false,
        "result",
      );
    default:
      return undefined;
  }
}

function piMessageUpdate(record: Record<string, unknown>): StepActivityBody | undefined {
  const event = record.assistantMessageEvent;
  if (!isRecord(event) || typeof event.delta !== "string" || event.delta.length === 0) {
    return undefined;
  }
  if (event.type === "text_delta") return { kind: "text", text: event.delta };
  if (event.type === "thinking_delta") return { kind: "thought", text: event.delta };
  return undefined;
}

function piToolExecution(
  record: Record<string, unknown>,
  status: StepActivityToolStatus,
  includeInput: boolean,
  outputField: "partialResult" | "result" | undefined,
): StepActivityBody | undefined {
  if (typeof record.toolCallId !== "string" || record.toolCallId.length === 0) return undefined;
  if (typeof record.toolName !== "string") return undefined;
  const tool: StepActivityTool = {
    toolCallId: record.toolCallId,
    title: record.toolName,
    kind: record.toolName,
    status,
  };
  if (includeInput) assignOptionalJson(tool, "input", record.args);
  if (outputField !== undefined) assignOptionalJson(tool, "output", record[outputField]);
  return { kind: "tool", text: "", tool };
}

function assignOptionalJson(
  tool: StepActivityTool,
  field: "input" | "output",
  value: unknown,
): void {
  const converted = tryJsonRoundTrip(value);
  if (converted !== undefined) tool[field] = converted;
}

function tryJsonRoundTrip(value: unknown): JsonValue | undefined {
  if (value === undefined) return undefined;
  try {
    const serialized = JSON.stringify(value);
    if (serialized === undefined) return undefined;
    return parseJson(serialized);
  } catch {
    return undefined;
  }
}

function stripTrailingCr(bytes: Buffer): Buffer {
  if (bytes.byteLength > 0 && bytes[bytes.byteLength - 1] === 0x0d) {
    return bytes.subarray(0, bytes.byteLength - 1);
  }
  return bytes;
}

function tryParseJsonLine(bytes: Buffer): { ok: true; value: unknown } | { ok: false } {
  if (bytes.byteLength === 0) return { ok: false };
  try {
    return { ok: true, value: JSON.parse(bytes.toString("utf8")) as unknown };
  } catch {
    return { ok: false };
  }
}

function normalizeMaxBatchBytes(value: number | undefined): number {
  if (value === undefined || !Number.isFinite(value)) return DEFAULT_MAX_BATCH_BYTES;
  return Math.max(MIN_BATCH_BYTES, Math.floor(value));
}

function defaultSetTimer(run: () => void, ms: number): unknown {
  const timer = setTimeout(run, ms);
  timer.unref?.();
  return timer;
}

function defaultClearTimer(handle: unknown): void {
  clearTimeout(handle as ReturnType<typeof setTimeout>);
}

function fitStatusBody(text: string, maxBatchBytes: number): StepActivityBody {
  const body: StepActivityBody = { kind: "status", text };
  if (canonicalBytes([body]) <= maxBatchBytes) return body;
  halveToFit(
    text,
    0,
    (value) => {
      body.text = value;
    },
    () => canonicalBytes([body]) <= maxBatchBytes,
  );
  return body;
}

/**
 * Shorten an oversized tool body to fit the protocol frame cap (`maxBatchBytes`). The complete tool
 * payload remains in the agent's own session record; only the live streaming copy is truncated.
 */
function fitToolBody(body: StepActivityBody, maxBatchBytes: number): StepActivityBody {
  const copy = cloneBody(body);
  if (canonicalBytes([copy]) <= maxBatchBytes) return copy;
  const tool = copy.tool;
  if (tool === undefined) return copy;
  tool.truncated = true;
  const fits = () => canonicalBytes([copy]) <= maxBatchBytes;
  if (shortenToolPayloads(tool, fits)) return copy;
  shortenToolStrings(copy, tool, fits);
  return copy;
}

function shortenToolPayloads(tool: StepActivityTool, fits: () => boolean): boolean {
  if (
    tool.output !== undefined &&
    halveToFit(canonicalJson(tool.output), 0, (value) => (tool.output = value), fits)
  ) {
    return true;
  }
  if (
    tool.input !== undefined &&
    halveToFit(canonicalJson(tool.input), 0, (value) => (tool.input = value), fits)
  ) {
    return true;
  }
  return false;
}

function shortenToolStrings(
  body: StepActivityBody,
  tool: StepActivityTool,
  fits: () => boolean,
): void {
  if (halveToFit(body.text, 0, (value) => (body.text = value), fits)) return;
  if (halveToFit(tool.title, 0, (value) => (tool.title = value), fits)) return;
  if (halveToFit(tool.toolCallId, 1, (value) => (tool.toolCallId = value), fits)) return;
  if (tool.kind !== undefined) {
    halveToFit(tool.kind, 0, (value) => (tool.kind = value), fits);
  }
}

function halveToFit(
  source: string,
  minLen: number,
  apply: (value: string) => void,
  fits: () => boolean,
): boolean {
  const floor = minLen > 0 ? firstCodePointLength(source) : 0;
  let kept = source.length;
  for (;;) {
    const end = Math.max(floor, clampSurrogateBoundary(source, 0, kept));
    apply(source.slice(0, end));
    if (fits()) return true;
    if (end <= floor) return false;
    kept = nextHalvedLength(kept, floor);
  }
}

function nextHalvedLength(kept: number, floor: number): number {
  const next = Math.floor(kept / 2);
  if (next <= floor || next === kept) return floor;
  return next;
}

function maxFittingCodePointLength(
  text: string,
  start: number,
  fits: (slice: string) => boolean,
  maxLen: number,
): number {
  let lo = 0;
  let hi = Math.min(text.length - start, maxLen);
  if (fits(text.slice(start, start + hi))) {
    return clampSurrogateBoundary(text, start, start + hi) - start;
  }
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    const end = clampSurrogateBoundary(text, start, start + mid);
    const len = end - start;
    if (len <= lo) {
      if (!fitsCodePointStep(text, start, lo, hi, fits)) break;
      lo += 2;
    } else if (fits(text.slice(start, end))) {
      lo = len;
    } else {
      hi = len - 1;
    }
  }
  return lo;
}

function fitsCodePointStep(
  text: string,
  start: number,
  lo: number,
  hi: number,
  fits: (slice: string) => boolean,
): boolean {
  return lo + 2 <= hi && fits(text.slice(start, start + lo + 2));
}

function firstCodePointLength(text: string): number {
  if (text.length === 0) return 0;
  return isSurrogatePairAt(text, 0) ? 2 : 1;
}

function clampSurrogateBoundary(text: string, start: number, end: number): number {
  if (end <= start || end >= text.length) return end;
  return isSurrogatePairAt(text, end - 1) ? end - 1 : end;
}

function isSurrogatePairAt(text: string, index: number): boolean {
  if (index < 0 || index + 1 >= text.length) return false;
  return isHighSurrogate(text.charCodeAt(index)) && isLowSurrogate(text.charCodeAt(index + 1));
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function sameToolCall(a: StepActivityBody | undefined, b: StepActivityBody): boolean {
  return (
    a?.kind === "tool" &&
    a.tool !== undefined &&
    b.tool !== undefined &&
    a.tool.toolCallId === b.tool.toolCallId
  );
}

function cloneBody(body: StepActivityBody): StepActivityBody {
  return body.tool === undefined
    ? { kind: body.kind, text: body.text }
    : { kind: body.kind, text: body.text, tool: { ...body.tool } };
}

function canonicalBytes(value: unknown): number {
  return Buffer.byteLength(canonicalJson(value), "utf8");
}

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function isToolStatus(value: unknown): value is StepActivityToolStatus {
  return (
    typeof value === "string" &&
    STEP_ACTIVITY_TOOL_STATUSES.includes(value as StepActivityToolStatus)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
```

=== FILE: test/step-activity.test.ts ===
```ts
import type { AcpRuntimeEvent } from "acpx/runtime";
import { describe, expect, it, vi } from "vitest";
import {
  JsonlRecordReader,
  StepActivityBatcher,
  StepActivityHub,
  stepActivityFrames,
  stepActivityFromAcpx,
  stepActivityFromPiRpc,
} from "../src/server/step-activity.js";
import { canonicalJson } from "../src/state/json.js";
import {
  STEP_ACTIVITY_KINDS,
  STEP_ACTIVITY_SCHEMA,
  parseStepActivity,
  parseStepActivityBodies,
  parseStepActivityBody,
  type StepActivity,
  type StepActivityBody,
} from "../src/state/step-activity.js";

function makeActivity(overrides: Partial<StepActivity> = {}): StepActivity {
  return {
    schema: STEP_ACTIVITY_SCHEMA,
    runId: "run-1",
    nodeId: "step-1",
    attemptId: "att-1",
    seq: 1,
    at: "2026-01-01T00:00:00.000Z",
    kind: "text",
    text: "hello",
    ...overrides,
  };
}

function byteSize(value: unknown): number {
  return Buffer.byteLength(canonicalJson(value), "utf8");
}

function hasLoneSurrogate(text: string): boolean {
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = text.charCodeAt(i + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return true;
      i += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true;
    }
  }
  return false;
}

describe("parseStepActivity", () => {
  it("round-trips valid activities of every kind without adding absent optional keys", () => {
    const samples: StepActivity[] = [
      makeActivity({ kind: "text", text: "" }),
      makeActivity({ kind: "thought", text: "reasoning", seq: 2 }),
      makeActivity({ kind: "status", text: "running checks", seq: 3 }),
      makeActivity({
        kind: "tool",
        text: "Read README.md",
        seq: 4,
        tool: { toolCallId: "call-1", title: "" },
      }),
      makeActivity({
        kind: "tool",
        text: "",
        seq: 5,
        tool: {
          toolCallId: "call-2",
          title: "bash",
          status: "completed",
          kind: "execute",
          input: { command: "ls" },
          output: { ok: true },
          truncated: true,
        },
      }),
    ];
    for (const sample of samples) {
      const parsed = parseStepActivity(JSON.parse(canonicalJson(sample)));
      expect(parsed).toEqual(sample);
      expect(canonicalJson(parsed)).toBe(canonicalJson(sample));
    }
    expect(STEP_ACTIVITY_KINDS).toEqual(["text", "thought", "tool", "status"]);
  });

  it("rejects extra keys, missing keys, bad schema, bad kind, mismatched tool, and invalid seq", () => {
    expect(() => parseStepActivity(null)).toThrow(/object/i);
    expect(() => parseStepActivity([])).toThrow(/object/i);
    expect(() => parseStepActivity({ ...makeActivity(), extra: 1 })).toThrow(/Unknown.*extra/);

    const missingRunId = { ...makeActivity() } as Record<string, unknown>;
    delete missingRunId.runId;
    expect(() => parseStepActivity(missingRunId)).toThrow(/Missing.*runId/);

    expect(() =>
      parseStepActivity({ ...makeActivity(), schema: "pi-workflows.step-activity.v999" }),
    ).toThrow(/schema/);
    expect(() => parseStepActivity({ ...makeActivity(), kind: "other" as never })).toThrow(/kind/);
    expect(() =>
      parseStepActivity({
        ...makeActivity({ kind: "text" }),
        tool: { toolCallId: "c1", title: "bash" },
      }),
    ).toThrow(/tool/);
    expect(() => parseStepActivity(makeActivity({ kind: "tool" }))).toThrow(/tool/);

    expect(() => parseStepActivity({ ...makeActivity(), seq: 0 })).toThrow(/seq/);
    expect(() => parseStepActivity({ ...makeActivity(), seq: -1 })).toThrow(/seq/);
    expect(() => parseStepActivity({ ...makeActivity(), seq: 1.5 })).toThrow(/seq/);
    expect(() => parseStepActivity({ ...makeActivity(), seq: "1" as never })).toThrow(/seq/);

    expect(() => parseStepActivity({ ...makeActivity(), runId: "" })).toThrow(/runId/);
    expect(() => parseStepActivity({ ...makeActivity(), nodeId: "" })).toThrow(/nodeId/);
    expect(() => parseStepActivity({ ...makeActivity(), attemptId: "" })).toThrow(/attemptId/);
    expect(() => parseStepActivity({ ...makeActivity(), at: "" })).toThrow(/at/);
  });
});

describe("parseStepActivityBody", () => {
  it("validates body and tool fields strictly while keeping absent optional fields absent", () => {
    const body: StepActivityBody = {
      kind: "tool",
      text: "",
      tool: {
        toolCallId: "call-1",
        title: "read",
        status: "in_progress",
        kind: "read",
        input: { path: "README.md" },
      },
    };
    const parsed = parseStepActivityBody(body);
    expect(parsed).toEqual(body);
    expect(Object.hasOwn(parsed.tool!, "output")).toBe(false);
    expect(Object.hasOwn(parsed.tool!, "truncated")).toBe(false);
    expect(canonicalJson(parsed)).toBe(canonicalJson(body));
  });

  it("rejects unknown or missing keys and invalid tool field types", () => {
    expect(() => parseStepActivityBody({ kind: "text", text: "ok", extra: true })).toThrow(
      /Unknown.*extra/,
    );
    expect(() => parseStepActivityBody({ kind: "text" })).toThrow(/Missing.*text/);
    expect(() => parseStepActivityBody({ kind: "text", text: 123 })).toThrow(/text/);
    expect(() => parseStepActivityBody({ kind: "text", text: "ok", tool: undefined })).toThrow(
      /tool/,
    );
    expect(() => parseStepActivityBody({ kind: "tool", text: "" })).toThrow(/tool/);
    expect(() =>
      parseStepActivityBody({ kind: "tool", text: "", tool: { toolCallId: "", title: "t" } }),
    ).toThrow(/toolCallId/);
    expect(() =>
      parseStepActivityBody({ kind: "tool", text: "", tool: { toolCallId: "1" } }),
    ).toThrow(/Missing.*title/);
    expect(() =>
      parseStepActivityBody({
        kind: "tool",
        text: "",
        tool: { toolCallId: "1", title: "t", unknown: 1 },
      }),
    ).toThrow(/Unknown.*unknown/);
    expect(() =>
      parseStepActivityBody({
        kind: "tool",
        text: "",
        tool: { toolCallId: "1", title: "t", status: "running" },
      }),
    ).toThrow(/tool\.status/);
    expect(() =>
      parseStepActivityBody({
        kind: "tool",
        text: "",
        tool: { toolCallId: "1", title: "t", kind: 42 },
      }),
    ).toThrow(/tool\.kind/);
    expect(() =>
      parseStepActivityBody({
        kind: "tool",
        text: "",
        tool: { toolCallId: "1", title: "t", truncated: "yes" },
      }),
    ).toThrow(/tool\.truncated/);
    expect(() =>
      parseStepActivityBody({
        kind: "tool",
        text: "",
        tool: { toolCallId: "1", title: "t", input: Number.NaN },
      }),
    ).toThrow(/tool\.input/);
    expect(() =>
      parseStepActivityBody({
        kind: "tool",
        text: "",
        tool: { toolCallId: "1", title: "t", output: undefined },
      }),
    ).toThrow(/tool\.output/);
  });
});

describe("parseStepActivityBodies", () => {
  it("accepts arrays of valid bodies and rejects non-arrays or invalid elements", () => {
    const bodies: StepActivityBody[] = [
      { kind: "text", text: "a" },
      { kind: "status", text: "working" },
    ];
    expect(parseStepActivityBodies([])).toEqual([]);
    expect(parseStepActivityBodies(bodies)).toEqual(bodies);
    expect(() => parseStepActivityBodies("not-an-array")).toThrow(/array/i);
    expect(() => parseStepActivityBodies([{ kind: "text", text: "ok" }, { kind: "bad" }])).toThrow(
      /Missing|kind/,
    );
  });
});

describe("stepActivityFromAcpx", () => {
  it("maps text_delta output and thought events and drops empty deltas", () => {
    expect(stepActivityFromAcpx({ type: "text_delta", text: "hello" })).toEqual({
      kind: "text",
      text: "hello",
    });
    expect(stepActivityFromAcpx({ type: "text_delta", stream: "output", text: "world" })).toEqual({
      kind: "text",
      text: "world",
    });
    expect(stepActivityFromAcpx({ type: "text_delta", stream: "thought", text: "hmm" })).toEqual({
      kind: "thought",
      text: "hmm",
    });
    expect(stepActivityFromAcpx({ type: "text_delta", text: "" })).toBeUndefined();
    expect(
      stepActivityFromAcpx({ type: "text_delta", stream: "thought", text: "" }),
    ).toBeUndefined();
  });

  it("maps plain status lines and ignores empty, usage, command-list, and plan status events", () => {
    expect(stepActivityFromAcpx({ type: "status", text: "Reading files" })).toEqual({
      kind: "status",
      text: "Reading files",
    });
    expect(stepActivityFromAcpx({ type: "status", text: "" })).toBeUndefined();
    expect(
      stepActivityFromAcpx({ type: "status", tag: "usage_update", text: "100 tokens" }),
    ).toBeUndefined();
    expect(
      stepActivityFromAcpx({ type: "status", tag: "available_commands_update", text: "/compact" }),
    ).toBeUndefined();
    expect(stepActivityFromAcpx({ type: "status", tag: "plan", text: "Step 1" })).toBeUndefined();
    expect(stepActivityFromAcpx({ type: "status", text: "ctx", used: 10 })).toBeUndefined();
    expect(stepActivityFromAcpx({ type: "status", text: "ctx", size: 100 })).toBeUndefined();
    expect(
      stepActivityFromAcpx({ type: "status", text: "cost", cost: { amount: 0.01 } }),
    ).toBeUndefined();
    expect(
      stepActivityFromAcpx({ type: "status", text: "tok", breakdown: { inputTokens: 5 } }),
    ).toBeUndefined();
    expect(
      stepActivityFromAcpx({
        type: "status",
        text: "cmds",
        availableCommands: [{ name: "compact" }],
      }),
    ).toBeUndefined();
    expect(
      stepActivityFromAcpx({
        type: "status",
        text: "plan",
        entries: [{ content: "a", status: "pending" }],
      }),
    ).toBeUndefined();
  });

  it("maps tool_call events with fallbacks and drops non-JSON inputs/outputs and unknown statuses", () => {
    expect(
      stepActivityFromAcpx({
        type: "tool_call",
        text: "Running ls",
        toolCallId: "tc-1",
        title: "Bash",
        status: "in_progress",
        kind: "execute",
        rawInput: { command: "ls" },
        rawOutput: { exitCode: 0 },
      }),
    ).toEqual({
      kind: "tool",
      text: "Running ls",
      tool: {
        toolCallId: "tc-1",
        title: "Bash",
        status: "in_progress",
        kind: "execute",
        input: { command: "ls" },
        output: { exitCode: 0 },
      },
    });

    // Fallback: missing toolCallId uses title; missing status/unknown status omitted.
    expect(
      stepActivityFromAcpx({
        type: "tool_call",
        text: "Opening file",
        title: "Read",
        status: "mystery",
      }),
    ).toEqual({
      kind: "tool",
      text: "Opening file",
      tool: { toolCallId: "Read", title: "Read" },
    });

    // Fallback: missing toolCallId and title uses "tool" for id and text for title.
    expect(stepActivityFromAcpx({ type: "tool_call", text: "Scanning" })).toEqual({
      kind: "tool",
      text: "Scanning",
      tool: { toolCallId: "tool", title: "Scanning" },
    });

    // Fallback: missing toolCallId, title, and text uses "tool" for both id and title.
    expect(stepActivityFromAcpx({ type: "tool_call", text: "" })).toEqual({
      kind: "tool",
      text: "",
      tool: { toolCallId: "tool", title: "tool" },
    });

    // Non-JSON rawInput (BigInt and circular object) and rawOutput are dropped cleanly.
    const cycle: Record<string, unknown> = {};
    cycle.self = cycle;
    const withBigInt = stepActivityFromAcpx({
      type: "tool_call",
      text: "call",
      toolCallId: "tc-2",
      title: "t",
      rawInput: BigInt(42),
      rawOutput: cycle,
    });
    expect(withBigInt).toEqual({
      kind: "tool",
      text: "call",
      tool: { toolCallId: "tc-2", title: "t" },
    });
    expect(Object.hasOwn(withBigInt!.tool!, "input")).toBe(false);
    expect(Object.hasOwn(withBigInt!.tool!, "output")).toBe(false);

    const withCycleInput = stepActivityFromAcpx({
      type: "tool_call",
      text: "call",
      toolCallId: "tc-3",
      title: "t",
      rawInput: cycle,
    });
    expect(Object.hasOwn(withCycleInput!.tool!, "input")).toBe(false);
  });

  it("maps done and error events to undefined", () => {
    expect(stepActivityFromAcpx({ type: "done", stopReason: "end_turn" })).toBeUndefined();
    expect(stepActivityFromAcpx({ type: "error", message: "boom" })).toBeUndefined();
    expect(stepActivityFromAcpx({ type: "unknown" } as unknown as AcpRuntimeEvent)).toBeUndefined();
  });
});

describe("stepActivityFromPiRpc", () => {
  it("maps text_delta, thinking_delta, and tool execution lifecycle records", () => {
    expect(
      stepActivityFromPiRpc({
        type: "message_update",
        usage: {},
        assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: "Hi " },
      }),
    ).toEqual({ kind: "text", text: "Hi " });

    expect(
      stepActivityFromPiRpc({
        type: "message_update",
        usage: {},
        assistantMessageEvent: { type: "thinking_delta", contentIndex: 0, delta: "Let me check" },
      }),
    ).toEqual({ kind: "thought", text: "Let me check" });

    expect(
      stepActivityFromPiRpc({
        type: "tool_execution_start",
        toolCallId: "call_1",
        toolName: "bash",
        args: { command: "ls -la" },
      }),
    ).toEqual({
      kind: "tool",
      text: "",
      tool: {
        toolCallId: "call_1",
        title: "bash",
        kind: "bash",
        status: "in_progress",
        input: { command: "ls -la" },
      },
    });

    expect(
      stepActivityFromPiRpc({
        type: "tool_execution_update",
        toolCallId: "call_1",
        toolName: "bash",
        args: { command: "ls -la" },
        partialResult: { content: [{ type: "text", text: "part" }] },
      }),
    ).toEqual({
      kind: "tool",
      text: "",
      tool: {
        toolCallId: "call_1",
        title: "bash",
        kind: "bash",
        status: "in_progress",
        input: { command: "ls -la" },
        output: { content: [{ type: "text", text: "part" }] },
      },
    });

    expect(
      stepActivityFromPiRpc({
        type: "tool_execution_end",
        toolCallId: "call_1",
        toolName: "bash",
        result: { content: [{ type: "text", text: "done" }] },
        isError: false,
      }),
    ).toEqual({
      kind: "tool",
      text: "",
      tool: {
        toolCallId: "call_1",
        title: "bash",
        kind: "bash",
        status: "completed",
        output: { content: [{ type: "text", text: "done" }] },
      },
    });

    expect(
      stepActivityFromPiRpc({
        type: "tool_execution_end",
        toolCallId: "call_1",
        toolName: "bash",
        result: { content: [{ type: "text", text: "failed" }] },
        isError: true,
      }),
    ).toEqual({
      kind: "tool",
      text: "",
      tool: {
        toolCallId: "call_1",
        title: "bash",
        kind: "bash",
        status: "failed",
        output: { content: [{ type: "text", text: "failed" }] },
      },
    });
  });

  it("returns undefined for unrelated or malformed records without ever throwing", () => {
    const throwingGetter = Object.defineProperty({}, "type", {
      get() {
        throw new Error("boom");
      },
    });
    const ignored = [
      null,
      undefined,
      42,
      "hello",
      [],
      throwingGetter,
      { type: "response", command: "prompt", success: true },
      { type: "agent_end", messages: [], willRetry: false },
      { type: "extension_ui_request", id: "1", method: "notify" },
      {
        type: "message_update",
        assistantMessageEvent: { type: "toolcall_delta", delta: "{}" },
      },
      {
        type: "message_update",
        assistantMessageEvent: { type: "text_delta", delta: "" },
      },
      {
        type: "message_update",
        assistantMessageEvent: { type: "text_delta", delta: 123 },
      },
      { type: "message_update", assistantMessageEvent: null },
      { type: "tool_execution_start", toolCallId: "", toolName: "bash" },
      { type: "tool_execution_start", toolCallId: 99, toolName: "bash" },
      { type: "tool_execution_start", toolCallId: "c1", toolName: null },
      { type: "tool_execution_end", toolCallId: "c1" },
    ];
    for (const record of ignored) {
      expect(stepActivityFromPiRpc(record)).toBeUndefined();
    }
  });
});

describe("JsonlRecordReader", () => {
  it("handles split chunks, CRLF, U+2028 inside JSON strings, invalid lines, and split UTF-8 bytes", () => {
    const reader = new JsonlRecordReader();
    // Split a single JSON record across chunks, with CRLF terminator.
    expect(reader.push('{"a":')).toEqual([]);
    expect(reader.push("1}\r\n")).toEqual([{ a: 1 }]);

    // U+2028 inside a JSON string must not split the record.
    const lineSeparatorRecord = `{"text":"before\u2028after"}\n`;
    expect(reader.push(lineSeparatorRecord)).toEqual([{ text: "before\u2028after" }]);

    // An invalid line between two valid ones is skipped.
    expect(reader.push('{"n":1}\nnot-json\n{"n":2}\n')).toEqual([{ n: 1 }, { n: 2 }]);

    // A 4-byte UTF-8 emoji split across two Buffer chunks.
    const emojiLine = Buffer.from('{"emoji":"😀"}\n', "utf8");
    const emojiStart = emojiLine.indexOf(0xf0);
    expect(reader.push(emojiLine.subarray(0, emojiStart + 2))).toEqual([]);
    expect(reader.push(emojiLine.subarray(emojiStart + 2))).toEqual([{ emoji: "😀" }]);
  });

  it("flushes a valid unterminated line on end() and ignores invalid or absent trailing data", () => {
    const valid = new JsonlRecordReader();
    expect(valid.push('{"last":true}')).toEqual([]);
    expect(valid.end()).toEqual([{ last: true }]);
    expect(valid.end()).toEqual([]);

    const invalid = new JsonlRecordReader();
    expect(invalid.push('{"broken":')).toEqual([]);
    expect(invalid.end()).toEqual([]);

    const empty = new JsonlRecordReader();
    expect(empty.push('{"done":1}\n')).toEqual([{ done: 1 }]);
    expect(empty.end()).toEqual([]);
  });
});

describe("StepActivityBatcher", () => {
  it("merges consecutive text and thought pushes while preserving cross-kind order", () => {
    const emitted: StepActivityBody[][] = [];
    const batcher = new StepActivityBatcher({ emit: (batch) => emitted.push(batch) });

    batcher.push({ kind: "text", text: "Hel" });
    batcher.push({ kind: "text", text: "lo" });
    batcher.push({ kind: "thought", text: "A" });
    batcher.push({ kind: "thought", text: "B" });
    batcher.push({ kind: "text", text: "!" });
    batcher.flush();

    expect(emitted).toEqual([
      [
        { kind: "text", text: "Hello" },
        { kind: "thought", text: "AB" },
        { kind: "text", text: "!" },
      ],
    ]);
  });

  it("replaces waiting status and matching tool bodies only when they are the last waiting item", () => {
    const emitted: StepActivityBody[][] = [];
    const batcher = new StepActivityBatcher({ emit: (batch) => emitted.push(batch) });

    batcher.push({ kind: "status", text: "step 1" });
    batcher.push({ kind: "status", text: "step 2" });
    batcher.push({ kind: "text", text: "middle" });
    batcher.push({ kind: "status", text: "step 3" });
    batcher.push({
      kind: "tool",
      text: "start",
      tool: { toolCallId: "t1", title: "bash", status: "in_progress" },
    });
    batcher.push({
      kind: "tool",
      text: "done",
      tool: { toolCallId: "t1", title: "bash", status: "completed" },
    });
    batcher.push({
      kind: "tool",
      text: "other",
      tool: { toolCallId: "t2", title: "read", status: "in_progress" },
    });
    // t1 is no longer the last waiting body, so this appends rather than replacing.
    batcher.push({
      kind: "tool",
      text: "late",
      tool: { toolCallId: "t1", title: "bash", status: "failed" },
    });
    batcher.flush();

    expect(emitted).toEqual([
      [
        { kind: "status", text: "step 2" },
        { kind: "text", text: "middle" },
        { kind: "status", text: "step 3" },
        {
          kind: "tool",
          text: "done",
          tool: { toolCallId: "t1", title: "bash", status: "completed" },
        },
        {
          kind: "tool",
          text: "other",
          tool: { toolCallId: "t2", title: "read", status: "in_progress" },
        },
        {
          kind: "tool",
          text: "late",
          tool: { toolCallId: "t1", title: "bash", status: "failed" },
        },
      ],
    ]);
  });

  it("arms one timer on the first push, disarms on flush and close, and runs no timer while idle", () => {
    const emitted: StepActivityBody[][] = [];
    const timers = new Map<number, () => void>();
    let nextId = 1;
    const setTimer = vi.fn((run: () => void, _ms: number) => {
      const id = nextId++;
      timers.set(id, run);
      return id;
    });
    const clearTimer = vi.fn((handle: unknown) => {
      timers.delete(handle as number);
    });

    const batcher = new StepActivityBatcher({
      emit: (batch) => emitted.push(batch),
      flushMs: 75,
      setTimer,
      clearTimer,
    });

    expect(setTimer).not.toHaveBeenCalled();
    batcher.push({ kind: "text", text: "a" });
    batcher.push({ kind: "text", text: "b" });
    expect(setTimer).toHaveBeenCalledTimes(1);
    expect(setTimer).toHaveBeenLastCalledWith(expect.any(Function), 75);

    // Firing the timer emits and leaves no active timer.
    const firstCallback = timers.get(1)!;
    timers.delete(1);
    firstCallback();
    expect(emitted).toEqual([[{ kind: "text", text: "ab" }]]);
    expect(timers.size).toBe(0);

    // Explicit flush disarms the pending timer.
    batcher.push({ kind: "text", text: "c" });
    expect(timers.size).toBe(1);
    batcher.flush();
    expect(clearTimer).toHaveBeenCalledTimes(1);
    expect(timers.size).toBe(0);
    expect(emitted).toEqual([[{ kind: "text", text: "ab" }], [{ kind: "text", text: "c" }]]);

    // close() flushes, disarms timer, and ignores later pushes.
    batcher.push({ kind: "text", text: "d" });
    expect(timers.size).toBe(1);
    batcher.close();
    expect(timers.size).toBe(0);
    expect(emitted).toHaveLength(3);
    batcher.push({ kind: "text", text: "ignored" });
    batcher.flush();
    expect(timers.size).toBe(0);
    expect(emitted).toHaveLength(3);
  });

  it("splits a 1,000,000-character text push within maxBatchBytes without losing or splitting surrogate pairs", () => {
    const maxBatchBytes = 2048;
    const emitted: StepActivityBody[][] = [];
    const batcher = new StepActivityBatcher({
      emit: (batch) => emitted.push(batch),
      maxBatchBytes,
    });

    const huge = "x".repeat(1_000_000);
    batcher.push({ kind: "text", text: huge });
    batcher.flush();

    expect(emitted.length).toBeGreaterThan(400);
    let rebuilt = "";
    for (const batch of emitted) {
      expect(byteSize(batch)).toBeLessThanOrEqual(maxBatchBytes);
      expect(parseStepActivityBodies(batch)).toEqual(batch);
      for (const body of batch) {
        expect(body.kind).toBe("text");
        rebuilt += body.text;
      }
    }
    expect(rebuilt).toBe(huge);

    // Astral-plane character positioned right across the 2048-byte boundary.
    const emojiBatches: StepActivityBody[][] = [];
    const emojiBatcher = new StepActivityBatcher({
      emit: (batch) => emojiBatches.push(batch),
      maxBatchBytes,
    });
    const overhead = byteSize([{ kind: "text", text: "" }]);
    const exactPrefix = "a".repeat(maxBatchBytes - overhead - 2);
    const withAstral = `${exactPrefix}😀tail`;
    emojiBatcher.push({ kind: "text", text: withAstral });
    emojiBatcher.flush();

    expect(emojiBatches.length).toBe(2);
    let rebuiltAstral = "";
    for (const batch of emojiBatches) {
      expect(byteSize(batch)).toBeLessThanOrEqual(maxBatchBytes);
      for (const body of batch) {
        expect(hasLoneSurrogate(body.text)).toBe(false);
        rebuiltAstral += body.text;
      }
    }
    expect(rebuiltAstral).toBe(withAstral);
  });

  it("shortens oversized tool bodies in the exact stage order while keeping batches within maxBatchBytes", () => {
    const maxBatchBytes = 1024;
    const runOne = (body: StepActivityBody): StepActivityBody => {
      const emitted: StepActivityBody[][] = [];
      const batcher = new StepActivityBatcher({
        emit: (batch) => emitted.push(batch),
        maxBatchBytes,
      });
      batcher.push(body);
      batcher.flush();
      expect(emitted).toHaveLength(1);
      expect(byteSize(emitted[0]!)).toBeLessThanOrEqual(maxBatchBytes);
      expect(parseStepActivityBodies(emitted[0]!)).toEqual(emitted[0]!);
      return emitted[0]![0]!;
    };

    // Stage 1: output shortened first; input remains untouched.
    const outputSource = { data: "o".repeat(2500) };
    const stage1 = runOne({
      kind: "tool",
      text: "run",
      tool: {
        toolCallId: "c1",
        title: "bash",
        input: { keep: true },
        output: outputSource,
      },
    });
    expect(stage1.tool!.truncated).toBe(true);
    expect(typeof stage1.tool!.output).toBe("string");
    expect(canonicalJson(outputSource).startsWith(stage1.tool!.output as string)).toBe(true);
    expect((stage1.tool!.output as string).length).toBeGreaterThan(0);
    expect(stage1.tool!.input).toEqual({ keep: true });

    // Stage 2: output hits "", then input is shortened; text remains untouched.
    const inputSource = { payload: "i".repeat(2500) };
    const stage2 = runOne({
      kind: "tool",
      text: "keep-text",
      tool: {
        toolCallId: "c2",
        title: "bash",
        input: inputSource,
        output: { small: 1 },
      },
    });
    expect(stage2.tool!.truncated).toBe(true);
    expect(stage2.tool!.output).toBe("");
    expect(typeof stage2.tool!.input).toBe("string");
    expect(canonicalJson(inputSource).startsWith(stage2.tool!.input as string)).toBe(true);
    expect((stage2.tool!.input as string).length).toBeGreaterThan(0);
    expect(stage2.text).toBe("keep-text");

    // Stage 3: text is shortened; title remains untouched.
    const stage3 = runOne({
      kind: "tool",
      text: "t".repeat(2500),
      tool: { toolCallId: "c3", title: "keep-title" },
    });
    expect(stage3.tool!.truncated).toBe(true);
    expect(stage3.text.length).toBeLessThan(2500);
    expect(stage3.text.length).toBeGreaterThan(0);
    expect(stage3.tool!.title).toBe("keep-title");

    // Stage 4: title is shortened; toolCallId remains untouched.
    const stage4 = runOne({
      kind: "tool",
      text: "",
      tool: { toolCallId: "keep-id", title: "T".repeat(2500) },
    });
    expect(stage4.tool!.truncated).toBe(true);
    expect(stage4.tool!.title.length).toBeLessThan(2500);
    expect(stage4.tool!.title.length).toBeGreaterThan(0);
    expect(stage4.tool!.toolCallId).toBe("keep-id");

    // Stage 5: toolCallId is shortened while keeping at least one character.
    const stage5 = runOne({
      kind: "tool",
      text: "",
      tool: { toolCallId: "I".repeat(2500), title: "" },
    });
    expect(stage5.tool!.truncated).toBe(true);
    expect(stage5.tool!.toolCallId.length).toBeLessThan(2500);
    expect(stage5.tool!.toolCallId.length).toBeGreaterThanOrEqual(1);
  });

  it("raises maxBatchBytes below 1024 to 1024 and keeps working if emit throws", () => {
    const received: StepActivityBody[][] = [];
    let failNext = true;
    const batcher = new StepActivityBatcher({
      maxBatchBytes: 64,
      emit: (batch) => {
        if (failNext) {
          failNext = false;
          throw new Error("socket closed");
        }
        received.push(batch);
      },
    });

    // A 500-byte body would exceed 64 but fits inside the raised 1024-byte floor.
    batcher.push({ kind: "text", text: "a".repeat(500) });
    expect(() => batcher.flush()).not.toThrow();

    batcher.push({ kind: "text", text: "b".repeat(500) });
    batcher.flush();
    expect(received).toEqual([[{ kind: "text", text: "b".repeat(500) }]]);
    expect(byteSize(received[0]!)).toBeLessThanOrEqual(1024);
  });
});

describe("StepActivityHub", () => {
  it("numbers records across attempts per run, isolates runs, replays, and restarts on closeRun", () => {
    let tick = 0;
    const hub = new StepActivityHub({
      now: () => new Date(Date.UTC(2026, 0, 1, 0, 0, tick++)),
    });

    expect(hub.lastSeq("run-1")).toBe(0);
    expect(hub.read("run-1", 0)).toEqual([]);
    expect(hub.publish({ runId: "run-1", nodeId: "n1", attemptId: "a1" }, [])).toEqual([]);
    expect(hub.lastSeq("run-1")).toBe(0);

    const first = hub.publish({ runId: "run-1", nodeId: "n1", attemptId: "a1" }, [
      { kind: "text", text: "one" },
      { kind: "text", text: "two" },
    ]);
    const second = hub.publish({ runId: "run-1", nodeId: "n2", attemptId: "a2" }, [
      { kind: "status", text: "next" },
    ]);
    const otherRun = hub.publish({ runId: "run-2", nodeId: "n1", attemptId: "b1" }, [
      { kind: "text", text: "isolated" },
    ]);

    expect(first.map((item) => item.seq)).toEqual([1, 2]);
    expect(second.map((item) => item.seq)).toEqual([3]);
    expect(second[0]!.attemptId).toBe("a2");
    expect(otherRun.map((item) => item.seq)).toEqual([1]);
    expect(hub.lastSeq("run-1")).toBe(3);
    expect(hub.read("run-1", 1).map((item) => item.seq)).toEqual([2, 3]);
    expect(hub.read("run-1", 3)).toEqual([]);

    for (const item of [...first, ...second, ...otherRun]) {
      expect(parseStepActivity(item)).toEqual(item);
    }

    // resumePoint on both sides of lastSeq.
    expect(hub.resumePoint("run-1", 0)).toEqual({ afterSeq: 0, reset: false });
    expect(hub.resumePoint("run-1", 3)).toEqual({ afterSeq: 3, reset: false });
    expect(hub.resumePoint("run-1", 4)).toEqual({ afterSeq: 0, reset: true });
    expect(hub.resumePoint("unknown", 1)).toEqual({ afterSeq: 0, reset: true });
    expect(hub.resumePoint("unknown", 0)).toEqual({ afterSeq: 0, reset: false });

    // closeRun forgets records and listeners, restarting numbering at 1.
    const closedListener = vi.fn();
    hub.subscribe("run-1", 3, closedListener);
    hub.closeRun("run-1");
    expect(hub.lastSeq("run-1")).toBe(0);
    expect(hub.read("run-1", 0)).toEqual([]);
    const restarted = hub.publish({ runId: "run-1", nodeId: "n1", attemptId: "a3" }, [
      { kind: "text", text: "fresh" },
    ]);
    expect(restarted[0]!.seq).toBe(1);
    expect(closedListener).not.toHaveBeenCalled();
  });

  it("handles replay, live delivery, idempotent unsubscribe, throwing listeners, and mid-delivery changes", () => {
    const hub = new StepActivityHub();
    const key = { runId: "run-1", nodeId: "n1", attemptId: "a1" };
    hub.publish(key, [{ kind: "text", text: "initial" }]);

    // Synchronous replay on subscribe when backlog is non-empty.
    const seen: number[][] = [];
    const unsubscribe = hub.subscribe("run-1", 0, (batch) => {
      seen.push(batch.map((item) => item.seq));
    });
    expect(seen).toEqual([[1]]);

    // No synchronous call when nothing newer exists.
    const quiet = vi.fn();
    const unsubQuiet = hub.subscribe("run-1", 1, quiet);
    expect(quiet).not.toHaveBeenCalled();

    // A throwing listener is removed and does not affect sibling listeners or publish().
    const broken = vi.fn(() => {
      throw new Error("listener exploded");
    });
    hub.subscribe("run-1", 1, broken);

    // A listener that adds another listener during delivery does not disturb delivery or double-deliver.
    const lateCalls: number[][] = [];
    let unsubLate: (() => void) | undefined;
    hub.subscribe("run-1", 1, () => {
      unsubLate ??= hub.subscribe("run-1", 0, (batch) => {
        lateCalls.push(batch.map((item) => item.seq));
      });
    });

    expect(() => hub.publish(key, [{ kind: "text", text: "second" }])).not.toThrow();
    expect(seen).toEqual([[1], [2]]);
    expect(quiet).toHaveBeenCalledTimes(1);
    expect(broken).toHaveBeenCalledTimes(1);
    expect(lateCalls).toEqual([[1, 2]]);

    // Unsubscribe is idempotent; broken listener stays removed.
    unsubscribe();
    unsubscribe();
    unsubQuiet();
    unsubLate?.();
    hub.publish(key, [{ kind: "text", text: "third" }]);
    expect(seen).toEqual([[1], [2]]);
    expect(quiet).toHaveBeenCalledTimes(1);
    expect(broken).toHaveBeenCalledTimes(1);
    expect(lateCalls).toEqual([[1, 2]]);
  });
});

describe("stepActivityFrames", () => {
  it("groups records within maxBytes in order, skips empty groups, and isolates oversized records", () => {
    expect(stepActivityFrames([], 1000)).toEqual([]);

    const small1 = makeActivity({ seq: 1, text: "a" });
    const small2 = makeActivity({ seq: 2, text: "b" });
    const huge = makeActivity({ seq: 3, text: "H".repeat(600) });
    const small3 = makeActivity({ seq: 4, text: "c" });

    const twoSmallBytes = byteSize([small1, small2]);
    const frames = stepActivityFrames([small1, small2, huge, small3], twoSmallBytes);

    expect(frames).toEqual([[small1, small2], [huge], [small3]]);
    expect(byteSize(frames[0]!)).toBeLessThanOrEqual(twoSmallBytes);
    expect(byteSize(frames[1]!)).toBeGreaterThan(twoSmallBytes);
    expect(byteSize(frames[2]!)).toBeLessThanOrEqual(twoSmallBytes);

    // One byte tighter forces small1 and small2 into separate frames.
    const tighter = stepActivityFrames([small1, small2], twoSmallBytes - 1);
    expect(tighter).toEqual([[small1], [small2]]);
  });
});
```