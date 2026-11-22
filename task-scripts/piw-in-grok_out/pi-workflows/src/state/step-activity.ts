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
