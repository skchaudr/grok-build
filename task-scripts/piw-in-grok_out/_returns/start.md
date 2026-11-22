=== FILE: src/viewer/start-command.ts ===
```ts
import { randomUUID } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import type { WorkflowClient } from "../client/client.js";
import type { ClientResponse } from "../client/protocol.js";
import type { ResolvedWorkflowLaunch } from "../client/resolver.js";
import { canonicalJson, parseJson, type JsonValue } from "../state/json.js";
import { errorMessage } from "../workflows/errors.js";
import { createRunId } from "../workflows/store.js";

const START_USAGE = `Usage:
  pi-workflows start <workflow file or name> [--input <json> | --input-file <path>] [--cwd <dir>] [--session <id>] [--json]`;

export interface StartCommandIo {
  stdout: (text: string) => void;
  stderr: (text: string) => void;
}

/**
 * The part of the workflow client this command uses: resolving the workflow,
 * ensuring the server connection is available, sending the durable `run.start`
 * command, reading the socket endpoint, and closing the client.
 */
export type StartCommandClient = Pick<
  WorkflowClient,
  "endpoint" | "ensureAvailable" | "resolveWorkflow" | "requestDurable" | "close"
>;

export interface StartCommandDeps {
  /**
   * Gives a client for the project at `cwd` with the server running.
   * Injected so tests can provide a fake client.
   */
  connect: (options: { cwd: string }) => Promise<StartCommandClient>;
  /** Reads `--input-file`. Default: node:fs/promises readFile as utf8. */
  readFile?: (path: string) => Promise<string>;
  now?: () => Date;
}

export type ParsedStartArgs = {
  workflowRef: string;
  /** From `--input`; absent when `--input-file` is used or neither is given. */
  input?: JsonValue;
  inputFile?: string;
  cwd: string;
  sessionId?: string;
  json: boolean;
};

type ArgParseState = {
  workflowRef?: string;
  input?: JsonValue;
  inputFile?: string;
  cwd?: string;
  sessionId?: string;
  json: boolean;
};

type StartResult = {
  runId: string;
  sessionId: string;
  workflowName: string;
  socketPath: string;
  revision?: number;
};

/**
 * Parsed arguments; exported for tests. Pure (reads no file). Throws Error
 * with a usage message on bad arguments.
 */
export function parseStartArgs(argv: string[]): ParsedStartArgs {
  const state: ArgParseState = { json: false };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg !== undefined) {
      index = consumeArg(argv, index, arg, state);
    }
  }
  return finalizeStartArgs(state);
}

/**
 * `pi-workflows start <workflow file or name> [--input <json> | --input-file <path>] [--cwd <dir>]
 *                     [--session <id>] [--json]`
 * Resolves the workflow, starts it headless, prints the run id. Returns the process exit code.
 */
export async function runStartCommand(
  argv: string[],
  io: StartCommandIo,
  deps: StartCommandDeps,
): Promise<number> {
  let parsed: ParsedStartArgs;
  try {
    parsed = parseStartArgs(argv);
  } catch (error) {
    io.stderr(`${errorMessage(error)}\n`);
    return 2;
  }
  try {
    const readFile = deps.readFile ?? defaultReadFile;
    const input = await loadStartInput(parsed, readFile);
    const result = await executeStart(parsed, input, deps);
    io.stdout(formatStartOutput(result, parsed.json));
    return 0;
  } catch (error) {
    io.stderr(`${errorMessage(error)}\n`);
    return 1;
  }
}

function consumeArg(argv: string[], index: number, arg: string, state: ArgParseState): number {
  if (arg === "--json") {
    state.json = true;
    return index;
  }
  if (arg.startsWith("-")) {
    return consumeOptionWithValue(argv, index, arg, state);
  }
  if (state.workflowRef !== undefined) {
    throw usageError(`Unexpected argument: ${arg}`);
  }
  if (arg.length === 0) {
    throw usageError("Workflow file or name cannot be empty");
  }
  state.workflowRef = arg;
  return index;
}

function consumeOptionWithValue(
  argv: string[],
  index: number,
  flag: string,
  state: ArgParseState,
): number {
  switch (flag) {
    case "--input":
      assignInlineInput(state, requireOptionValue(argv, index, flag));
      return index + 1;
    case "--input-file":
      assignUniqueTextOption(
        state,
        "inputFile",
        requireNonEmptyOptionValue(argv, index, flag),
        flag,
      );
      return index + 1;
    case "--cwd":
      assignUniqueTextOption(state, "cwd", requireNonEmptyOptionValue(argv, index, flag), flag);
      return index + 1;
    case "--session":
      assignUniqueTextOption(
        state,
        "sessionId",
        requireNonEmptyOptionValue(argv, index, flag),
        flag,
      );
      return index + 1;
    default:
      throw usageError(`Unknown argument: ${flag}`);
  }
}

function assignInlineInput(state: ArgParseState, raw: string): void {
  if (state.input !== undefined) {
    throw usageError("--input may only be specified once");
  }
  state.input = parseInlineInput(raw);
}

function assignUniqueTextOption(
  state: ArgParseState,
  field: "inputFile" | "cwd" | "sessionId",
  value: string,
  flag: string,
): void {
  if (state[field] !== undefined) {
    throw usageError(`${flag} may only be specified once`);
  }
  state[field] = value;
}

function requireOptionValue(argv: string[], index: number, flag: string): string {
  const value = argv[index + 1];
  if (value === undefined) {
    throw usageError(`${flag} requires a value`);
  }
  return value;
}

function requireNonEmptyOptionValue(argv: string[], index: number, flag: string): string {
  const value = requireOptionValue(argv, index, flag);
  if (value.trim().length === 0) {
    throw usageError(`${flag} requires a nonempty value`);
  }
  return value;
}

function parseInlineInput(raw: string): JsonValue {
  try {
    return parseJson(raw);
  } catch (error) {
    throw usageError(`Invalid JSON for --input: ${errorMessage(error)}`);
  }
}

function finalizeStartArgs(state: ArgParseState): ParsedStartArgs {
  if (state.workflowRef === undefined) {
    throw usageError("start requires a workflow file or name");
  }
  if (state.input !== undefined && state.inputFile !== undefined) {
    throw usageError("Cannot use both --input and --input-file");
  }
  return buildParsedStartArgs(state.workflowRef, state);
}

function buildParsedStartArgs(workflowRef: string, state: ArgParseState): ParsedStartArgs {
  const parsed: ParsedStartArgs = {
    workflowRef,
    cwd: path.resolve(state.cwd ?? process.cwd()),
    json: state.json,
  };
  if (state.input !== undefined) parsed.input = state.input;
  if (state.inputFile !== undefined) parsed.inputFile = state.inputFile;
  if (state.sessionId !== undefined) parsed.sessionId = state.sessionId;
  return parsed;
}

async function loadStartInput(
  parsed: ParsedStartArgs,
  readFile: (filePath: string) => Promise<string>,
): Promise<JsonValue> {
  if (parsed.input !== undefined) return parsed.input;
  if (parsed.inputFile === undefined) return {};
  const raw = await readInputFile(parsed.inputFile, readFile);
  return parseJsonValue(raw, `--input-file ${parsed.inputFile}`);
}

async function readInputFile(
  filePath: string,
  readFile: (target: string) => Promise<string>,
): Promise<string> {
  try {
    return await readFile(filePath);
  } catch (error) {
    throw new Error(`Could not read --input-file ${filePath}: ${errorMessage(error)}`);
  }
}

function parseJsonValue(raw: string, source: string): JsonValue {
  try {
    return parseJson(raw);
  } catch (error) {
    throw new Error(`Invalid JSON in ${source}: ${errorMessage(error)}`);
  }
}

async function executeStart(
  parsed: ParsedStartArgs,
  input: JsonValue,
  deps: StartCommandDeps,
): Promise<StartResult> {
  const client = await deps.connect({ cwd: parsed.cwd });
  try {
    const resolved = await client.resolveWorkflow({
      cwd: parsed.cwd,
      workflowRef: parsed.workflowRef,
    });
    const now = deps.now === undefined ? new Date() : deps.now();
    const runId = createRunId(resolved.workflowName, now);
    const sessionId = parsed.sessionId ?? `cli-session-${randomUUID()}`;
    const response = await sendStartRequest(client, {
      runId,
      sessionId,
      projectPath: parsed.cwd,
      resolved,
      input,
    });
    return {
      runId,
      sessionId,
      workflowName: resolved.workflowName,
      socketPath: client.endpoint,
      ...(response.revision === undefined ? {} : { revision: response.revision }),
    };
  } finally {
    await client.close();
  }
}

async function sendStartRequest(
  client: StartCommandClient,
  options: {
    runId: string;
    sessionId: string;
    projectPath: string;
    resolved: ResolvedWorkflowLaunch;
    input: JsonValue;
  },
): Promise<ClientResponse> {
  await client.ensureAvailable();
  const idempotencyKey = randomUUID();
  const response = await client.requestDurable({
    operation: "run.start",
    requestId: `start-${idempotencyKey}`,
    idempotencyKey,
    runId: options.runId,
    payload: {
      projectPath: options.projectPath,
      workflowName: options.resolved.workflowName,
      workflowSourceRef: options.resolved.workflowSourceRef,
      workflowSource: options.resolved.workflowSource,
      definitionDigest: options.resolved.definitionDigest,
      definitionSnapshot: options.resolved.definitionSnapshot,
      input: options.input,
      launchOptions: {},
      originSessionId: options.sessionId,
      executionMode: "headless",
    },
  });
  if (response.outcome !== "accepted" && response.outcome !== "adopted") {
    throw new Error(response.error ?? `Workflow server rejected run.start (${response.outcome})`);
  }
  return response;
}

function formatStartOutput(result: StartResult, json: boolean): string {
  if (json) {
    return `${canonicalJson(result)}\n`;
  }
  return `Started workflow ${result.workflowName} as ${result.runId}.\n`;
}

async function defaultReadFile(filePath: string): Promise<string> {
  return await fs.readFile(filePath, "utf8");
}

function usageError(message: string): Error {
  return new Error(`${message}\n\n${START_USAGE}`);
}
```
=== FILE: test/start-command.test.ts ===
```ts
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { CLIENT_PROTOCOL_SCHEMA, type ClientResponse } from "../src/client/protocol.js";
import type { ResolvedWorkflowLaunch } from "../src/client/resolver.js";
import { canonicalJson } from "../src/state/json.js";
import {
  parseStartArgs,
  runStartCommand,
  type StartCommandClient,
  type StartCommandDeps,
  type StartCommandIo,
} from "../src/viewer/start-command.js";

const SOCKET_PATH = "/tmp/pi-workflows-test/server/server.sock";
const FIXED_NOW = new Date("2026-04-16T12:34:56.000Z");

const RESOLVED_LAUNCH = {
  schema: "pi-workflows.resolved-launch.v1",
  workflowName: "ship-feature",
  workflowSourceRef: "/project/ship-feature.workflow.ts",
  workflowSource: {
    kind: "file",
    path: "/project/ship-feature.workflow.ts",
    hash: "abc123",
  },
  definitionDigest: "sha256:def456",
  definitionSnapshot: {
    schema: "pi-workflows.definition-snapshot.v1",
    name: "ship-feature",
    startAt: "plan",
    nodes: { plan: { kind: "agent" } },
    edges: [],
  },
} as unknown as ResolvedWorkflowLaunch;

function makeIo(): { io: StartCommandIo; stdout: () => string; stderr: () => string } {
  let out = "";
  let err = "";
  return {
    io: {
      stdout: (text) => {
        out += text;
      },
      stderr: (text) => {
        err += text;
      },
    },
    stdout: () => out,
    stderr: () => err,
  };
}

function acceptedResponse(revision?: number): ClientResponse {
  return {
    schema: CLIENT_PROTOCOL_SCHEMA,
    type: "response",
    requestId: "req-1",
    outcome: "accepted",
    ...(revision === undefined ? {} : { revision }),
    receipt: { runId: "ship-feature-1", status: "queued" },
  };
}

function makeFakeClient(overrides: Partial<StartCommandClient> = {}) {
  const ensureAvailable = vi.fn(async () => ({
    schema: CLIENT_PROTOCOL_SCHEMA,
    type: "hello" as const,
    connectionId: "conn-1",
    packageVersion: "0.0.0-test",
  }));
  const resolveWorkflow = vi.fn(async () => RESOLVED_LAUNCH);
  const requestDurable = vi.fn(async () => acceptedResponse(1));
  const close = vi.fn(async () => undefined);
  const client: StartCommandClient = {
    endpoint: SOCKET_PATH,
    ensureAvailable,
    resolveWorkflow,
    requestDurable,
    close,
    ...overrides,
  };
  return { client, ensureAvailable, resolveWorkflow, requestDurable, close };
}

describe("parseStartArgs", () => {
  it("parses a workflow reference with defaults", () => {
    const parsed = parseStartArgs(["my-workflow"]);
    expect(parsed).toEqual({
      workflowRef: "my-workflow",
      cwd: process.cwd(),
      json: false,
    });
    expect("input" in parsed).toBe(false);
    expect("inputFile" in parsed).toBe(false);
    expect("sessionId" in parsed).toBe(false);
  });

  it("parses --input, --cwd, --session, and --json in any order", () => {
    const parsed = parseStartArgs([
      "--json",
      "--cwd",
      "/tmp/project",
      "--session",
      "grok-session-1",
      "deploy.workflow.ts",
      "--input",
      '{"b":2,"a":1}',
    ]);
    expect(parsed).toEqual({
      workflowRef: "deploy.workflow.ts",
      input: { a: 1, b: 2 },
      cwd: path.resolve("/tmp/project"),
      sessionId: "grok-session-1",
      json: true,
    });
    expect("inputFile" in parsed).toBe(false);
  });

  it("parses primitive JSON values for --input, including negative numbers and null", () => {
    expect(parseStartArgs(["wf", "--input", "-42"]).input).toBe(-42);
    expect(parseStartArgs(["wf", "--input", "null"]).input).toBeNull();
    expect(parseStartArgs(["wf", "--input", "false"]).input).toBe(false);
  });

  it("parses --input-file without reading the file and omits input", () => {
    const parsed = parseStartArgs(["wf.workflow.ts", "--input-file", "missing-file.json"]);
    expect(parsed).toEqual({
      workflowRef: "wf.workflow.ts",
      inputFile: "missing-file.json",
      cwd: process.cwd(),
      json: false,
    });
    expect("input" in parsed).toBe(false);
  });

  it("resolves a relative --cwd to an absolute path", () => {
    const parsed = parseStartArgs(["wf", "--cwd", "relative/dir"]);
    expect(parsed.cwd).toBe(path.resolve("relative/dir"));
  });

  it("rejects both --input and --input-file with usage", () => {
    expect(() =>
      parseStartArgs(["wf", "--input", "{}", "--input-file", "in.json"]),
    ).toThrow(/Cannot use both --input and --input-file[\s\S]*Usage:/);
  });

  it("rejects a missing workflow with usage", () => {
    expect(() => parseStartArgs([])).toThrow(/start requires a workflow[\s\S]*Usage:/);
    expect(() => parseStartArgs(["--json"])).toThrow(/start requires a workflow[\s\S]*Usage:/);
  });

  it("rejects an unknown flag with usage", () => {
    expect(() => parseStartArgs(["wf", "--bogus"])).toThrow(/Unknown argument: --bogus[\s\S]*Usage:/);
  });

  it.each(["--input", "--input-file", "--cwd", "--session"])(
    "rejects %s without a value",
    (flag) => {
      expect(() => parseStartArgs(["wf", flag])).toThrow(/requires a value[\s\S]*Usage:/);
    },
  );

  it.each([
    [["wf", "--input", "{bad"]],
    [["wf", "--input", ""]],
  ])("rejects invalid --input JSON: %j", (argv) => {
    expect(() => parseStartArgs(argv)).toThrow(/Invalid JSON for --input[\s\S]*Usage:/);
  });

  it("rejects duplicate options, extra positionals, and empty values", () => {
    expect(() => parseStartArgs(["wf", "extra"])).toThrow(/Unexpected argument: extra[\s\S]*Usage:/);
    expect(() => parseStartArgs([""])).toThrow(/cannot be empty[\s\S]*Usage:/);
    expect(() => parseStartArgs(["wf", "--session", "   "])).toThrow(
      /requires a nonempty value[\s\S]*Usage:/,
    );
    expect(() => parseStartArgs(["wf", "--input", "1", "--input", "2"])).toThrow(
      /--input may only be specified once[\s\S]*Usage:/,
    );
    expect(() => parseStartArgs(["wf", "--cwd", "/a", "--cwd", "/b"])).toThrow(
      /--cwd may only be specified once[\s\S]*Usage:/,
    );
  });
});

describe("runStartCommand", () => {
  it("returns exit code 2 with usage on stderr for bad arguments without connecting", async () => {
    const capture = makeIo();
    const connect = vi.fn(async () => makeFakeClient().client);
    const code = await runStartCommand(["--unknown"], capture.io, { connect });
    expect(code).toBe(2);
    expect(capture.stdout()).toBe("");
    expect(capture.stderr()).toContain("Unknown argument: --unknown");
    expect(capture.stderr()).toContain("Usage:");
    expect(connect).not.toHaveBeenCalled();
  });

  it("returns exit code 1 for an unreadable --input-file without connecting", async () => {
    const capture = makeIo();
    const connect = vi.fn(async () => makeFakeClient().client);
    const readFile = vi.fn(async () => {
      throw new Error("ENOENT: no such file");
    });
    const code = await runStartCommand(["wf", "--input-file", "/nope.json"], capture.io, {
      connect,
      readFile,
    });
    expect(code).toBe(1);
    expect(capture.stdout()).toBe("");
    expect(capture.stderr()).toContain("ENOENT: no such file");
    expect(connect).not.toHaveBeenCalled();
  });

  it("returns exit code 1 for a non-JSON --input-file without connecting", async () => {
    const capture = makeIo();
    const connect = vi.fn(async () => makeFakeClient().client);
    const readFile = vi.fn(async () => "not json at all");
    const code = await runStartCommand(["wf", "--input-file", "bad.json"], capture.io, {
      connect,
      readFile,
    });
    expect(code).toBe(1);
    expect(capture.stdout()).toBe("");
    expect(capture.stderr()).toContain("Invalid JSON in --input-file bad.json");
    expect(connect).not.toHaveBeenCalled();
  });

  it("starts a headless run, sends the exact run.start payload, outputs canonical JSON, and closes", async () => {
    const capture = makeIo();
    const fake = makeFakeClient();
    const connect = vi.fn(async () => fake.client);
    const now = vi.fn(() => FIXED_NOW);
    const projectDir = "/tmp/my-project";

    const code = await runStartCommand(
      [
        "ship-feature.workflow.ts",
        "--cwd",
        projectDir,
        "--input",
        '{"task":"add start command","count":2}',
        "--session",
        "grok-build-42",
        "--json",
      ],
      capture.io,
      { connect, now },
    );

    expect(code).toBe(0);
    expect(capture.stderr()).toBe("");
    expect(connect).toHaveBeenCalledWith({ cwd: path.resolve(projectDir) });
    expect(fake.resolveWorkflow).toHaveBeenCalledWith({
      cwd: path.resolve(projectDir),
      workflowRef: "ship-feature.workflow.ts",
    });
    expect(fake.ensureAvailable).toHaveBeenCalledTimes(1);
    expect(now).toHaveBeenCalledTimes(1);
    expect(fake.close).toHaveBeenCalledTimes(1);

    const request = fake.requestDurable.mock.calls[0]?.[0];
    expect(request).toBeDefined();
    if (request === undefined) return;

    expect(request.operation).toBe("run.start");
    expect(typeof request.runId).toBe("string");
    expect(request.runId).toContain("ship-feature");
    expect(request.requestId).toBe(`start-${request.idempotencyKey}`);
    expect(request.payload).toEqual({
      projectPath: path.resolve(projectDir),
      workflowName: RESOLVED_LAUNCH.workflowName,
      workflowSourceRef: RESOLVED_LAUNCH.workflowSourceRef,
      workflowSource: RESOLVED_LAUNCH.workflowSource,
      definitionDigest: RESOLVED_LAUNCH.definitionDigest,
      definitionSnapshot: RESOLVED_LAUNCH.definitionSnapshot,
      input: { count: 2, task: "add start command" },
      launchOptions: {},
      originSessionId: "grok-build-42",
      executionMode: "headless",
    });

    const lines = capture.stdout().trimEnd().split("\n");
    expect(lines).toHaveLength(1);
    const line = lines[0];
    expect(line).toBeDefined();
    if (line === undefined) return;
    const expectedOutput = {
      runId: request.runId,
      sessionId: "grok-build-42",
      workflowName: "ship-feature",
      socketPath: SOCKET_PATH,
      revision: 1,
    };
    expect(line).toBe(canonicalJson(expectedOutput));
  });

  it("prints one human line without --json and generates a fresh session id when --session is omitted", async () => {
    const capture = makeIo();
    const fake = makeFakeClient();
    const code = await runStartCommand(["ship-feature"], capture.io, {
      connect: async () => fake.client,
      now: () => FIXED_NOW,
    });

    expect(code).toBe(0);
    expect(capture.stderr()).toBe("");
    const request = fake.requestDurable.mock.calls[0]?.[0];
    expect(request).toBeDefined();
    if (request === undefined) return;

    const payload = request.payload as Record<string, unknown>;
    expect(typeof payload.originSessionId).toBe("string");
    expect((payload.originSessionId as string).length).toBeGreaterThan(0);
    expect(payload.input).toEqual({});
    expect(payload.executionMode).toBe("headless");

    const output = capture.stdout();
    expect(output.endsWith("\n")).toBe(true);
    expect(output.trimEnd().split("\n")).toHaveLength(1);
    expect(output).toContain(request.runId as string);
    expect(output).toContain("ship-feature");
    expect(fake.close).toHaveBeenCalledTimes(1);
  });

  it("reads --input-file from disk with the default reader", async () => {
    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "pi-workflows-start-test-"));
    try {
      const inputPath = path.join(tempDir, "input.json");
      await fs.writeFile(inputPath, '{"goal":"verify","steps":[1,2]}\n', "utf8");
      const capture = makeIo();
      const fake = makeFakeClient();
      const code = await runStartCommand(
        ["ship-feature", "--input-file", inputPath, "--cwd", tempDir],
        capture.io,
        { connect: async () => fake.client },
      );

      expect(code).toBe(0);
      const request = fake.requestDurable.mock.calls[0]?.[0];
      expect(request).toBeDefined();
      if (request === undefined) return;
      expect((request.payload as Record<string, unknown>).input).toEqual({
        goal: "verify",
        steps: [1, 2],
      });
      expect(fake.close).toHaveBeenCalledTimes(1);
    } finally {
      await fs.rm(tempDir, { recursive: true, force: true });
    }
  });

  it("accepts an adopted response and omits revision when absent", async () => {
    const capture = makeIo();
    const fake = makeFakeClient({
      requestDurable: vi.fn(async () => ({
        schema: CLIENT_PROTOCOL_SCHEMA,
        type: "response" as const,
        requestId: "req-1",
        outcome: "adopted" as const,
        receipt: { runId: "ship-feature-1", status: "queued" },
      })),
    });

    const code = await runStartCommand(
      ["ship-feature", "--session", "sess-adopted", "--json"],
      capture.io,
      { connect: async () => fake.client },
    );

    expect(code).toBe(0);
    const parsed = JSON.parse(capture.stdout()) as Record<string, unknown>;
    expect(parsed.sessionId).toBe("sess-adopted");
    expect(parsed.workflowName).toBe("ship-feature");
    expect(parsed.socketPath).toBe(SOCKET_PATH);
    expect("revision" in parsed).toBe(false);
    expect(fake.close).toHaveBeenCalledTimes(1);
  });

  it("returns exit code 1 with the loader error and closes the client on resolve failure", async () => {
    const capture = makeIo();
    const fake = makeFakeClient({
      resolveWorkflow: vi.fn(async () => {
        throw new Error('Unknown workflow "missing". Available workflows: ship-feature');
      }),
    });

    const code = await runStartCommand(["missing"], capture.io, {
      connect: async () => fake.client,
    });

    expect(code).toBe(1);
    expect(capture.stdout()).toBe("");
    expect(capture.stderr()).toContain('Unknown workflow "missing"');
    expect(fake.requestDurable).not.toHaveBeenCalled();
    expect(fake.close).toHaveBeenCalledTimes(1);
  });

  it("returns exit code 1 with the server error and closes the client on server refusal", async () => {
    const capture = makeIo();
    const fake = makeFakeClient({
      requestDurable: vi.fn(async () => ({
        schema: CLIENT_PROTOCOL_SCHEMA,
        type: "response" as const,
        requestId: "req-1",
        outcome: "rejected" as const,
        error: "Workflow run project is missing",
      })),
    });

    const code = await runStartCommand(["ship-feature"], capture.io, {
      connect: async () => fake.client,
    });

    expect(code).toBe(1);
    expect(capture.stdout()).toBe("");
    expect(capture.stderr()).toContain("Workflow run project is missing");
    expect(fake.close).toHaveBeenCalledTimes(1);
  });

  it("returns exit code 1 with a fallback message when refusal has no error detail", async () => {
    const capture = makeIo();
    const fake = makeFakeClient({
      requestDurable: vi.fn(async () => ({
        schema: CLIENT_PROTOCOL_SCHEMA,
        type: "response" as const,
        requestId: "req-1",
        outcome: "unavailable" as const,
      })),
    });

    const code = await runStartCommand(["ship-feature"], capture.io, {
      connect: async () => fake.client,
    });

    expect(code).toBe(1);
    expect(capture.stderr()).toContain("unavailable");
    expect(fake.close).toHaveBeenCalledTimes(1);
  });

  it("returns exit code 1 when the server cannot be reached during connect or ensureAvailable", async () => {
    const connectFailIo = makeIo();
    const deps: StartCommandDeps = {
      connect: async () => {
        throw new Error("Workflow server did not become ready: socket refused");
      },
    };
    expect(await runStartCommand(["ship-feature"], connectFailIo.io, deps)).toBe(1);
    expect(connectFailIo.stderr()).toContain("socket refused");

    const unavailableIo = makeIo();
    const fake = makeFakeClient({
      ensureAvailable: vi.fn(async () => {
        throw new Error("Workflow server connection timed out");
      }),
    });
    expect(
      await runStartCommand(["ship-feature"], unavailableIo.io, {
        connect: async () => fake.client,
      }),
    ).toBe(1);
    expect(unavailableIo.stderr()).toContain("Workflow server connection timed out");
    expect(fake.close).toHaveBeenCalledTimes(1);
  });
});
```