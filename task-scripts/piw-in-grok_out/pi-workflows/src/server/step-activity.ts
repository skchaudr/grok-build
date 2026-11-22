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
