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
