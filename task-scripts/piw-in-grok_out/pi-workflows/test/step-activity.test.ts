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
