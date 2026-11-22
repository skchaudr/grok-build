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
