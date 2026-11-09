/**
 * Pi spawn executor for upstream xai-workflow host (grok-pi).
 *
 * Does NOT interpret Rhai. The Rust host runs xai-workflow; this extension only
 * implements SpawnAgent via createAgentSession (same kernel as pi-grok-subagents).
 *
 * Bridge protocol (file-based, matches hidden command style):
 *   /__pi_workflow_spawn --request <path> --response <path>
 *   /__pi_workflow_cancel --run-id <id>
 */
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import {
  createAgentSession,
  DefaultResourceLoader,
  getAgentDir,
  SessionManager,
  SettingsManager,
  type AgentSession,
  type ExtensionAPI,
  type ExtensionContext,
} from "@earendil-works/pi-coding-agent";

const SPAWN_COMMAND = "__pi_workflow_spawn";
const CANCEL_COMMAND = "__pi_workflow_cancel";
const BRIDGE_TYPE = "pi-grok-workflow/v1";

type CapabilityMode = "read-only" | "read-write" | "execute" | "all";

const CAPABILITY_TOOLS: Record<CapabilityMode, string[]> = {
  "read-only": ["read", "grep", "find", "ls"],
  "read-write": ["read", "grep", "find", "ls", "edit", "write"],
  execute: ["read", "grep", "find", "ls", "edit", "write", "bash"],
  all: ["read", "grep", "find", "ls", "edit", "write", "bash"],
};

type SpawnRequest = {
  id: string;
  prompt: string;
  description?: string;
  subagent_type?: string;
  parent_session_id?: string;
  resume_from?: string;
  model?: string;
  capability_mode?: string;
  isolation_worktree?: boolean;
  fork_context?: boolean;
  run_id: string;
};

type SpawnResponse = {
  success: boolean;
  output: string;
  error?: string;
  cancelled: boolean;
  child_session_id: string;
  total_tokens_used: number;
  duration_ms: number;
  backgrounded: boolean;
};

const activeByRun = new Map<string, Set<AgentSession>>();

function parseArgs(args: string): Record<string, string> {
  const out: Record<string, string> = {};
  const tokens = args.trim().split(/\s+/).filter(Boolean);
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (!t.startsWith("--")) continue;
    const key = t.slice(2);
    const val = tokens[i + 1];
    if (val && !val.startsWith("--")) {
      out[key] = val;
      i++;
    } else {
      out[key] = "1";
    }
  }
  return out;
}

function normalizeCapability(raw: string | undefined): CapabilityMode {
  const v = (raw ?? "all").toLowerCase();
  if (v === "read-only" || v === "read-write" || v === "execute" || v === "all") {
    return v;
  }
  return "all";
}

function lastAssistantText(session: AgentSession): string {
  const branch = session.messages ?? [];
  for (let i = branch.length - 1; i >= 0; i--) {
    const m = branch[i] as { role?: string; content?: unknown };
    if (m.role !== "assistant") continue;
    if (typeof m.content === "string") return m.content;
    if (Array.isArray(m.content)) {
      return m.content
        .map((b) => {
          if (typeof b === "string") return b;
          if (b && typeof b === "object" && "text" in b) {
            return String((b as { text?: string }).text ?? "");
          }
          return "";
        })
        .join("");
    }
  }
  return "";
}

function writeResponse(path: string, body: SpawnResponse): void {
  writeFileSync(path, JSON.stringify(body), "utf8");
}

export default function (pi: ExtensionAPI): void {
  if (process.env.PI_GROK !== "1" && process.env.PI_GROK_WORKFLOWS !== "1") {
    return;
  }

  pi.registerCommand(SPAWN_COMMAND, {
    description: "Internal: spawn workflow agent for xai-workflow host",
    hidden: true,
    handler: async (args, ctx) => {
      const parsed = parseArgs(args);
      const requestPath = parsed.request;
      const responsePath = parsed.response;
      if (!requestPath || !responsePath) {
        ctx.ui.notify("workflow spawn requires --request and --response", "error");
        return;
      }

      let request: SpawnRequest;
      try {
        request = JSON.parse(readFileSync(requestPath, "utf8")) as SpawnRequest;
      } catch (e) {
        writeResponse(responsePath, {
          success: false,
          output: "",
          error: `invalid request file: ${e instanceof Error ? e.message : String(e)}`,
          cancelled: false,
          child_session_id: "",
          total_tokens_used: 0,
          duration_ms: 0,
          backgrounded: false,
        });
        return;
      }

      const started = Date.now();
      const id = request.id || randomUUID();
      const capabilityMode = normalizeCapability(request.capability_mode);
      const model = ctx.model;
      if (!model) {
        writeResponse(responsePath, {
          success: false,
          output: "",
          error: "no Pi model is selected",
          cancelled: false,
          child_session_id: "",
          total_tokens_used: 0,
          duration_ms: Date.now() - started,
          backgrounded: false,
        });
        return;
      }

      try {
        const agentDir = getAgentDir();
        const settingsManager = SettingsManager.create(ctx.cwd, agentDir);
        const resourceLoader = new DefaultResourceLoader({
          cwd: ctx.cwd,
          agentDir,
          noExtensions: true,
          noSkills: true,
          noPromptTemplates: true,
          noThemes: true,
          noContextFiles: true,
          systemPromptOverride: () =>
            "You are a focused workflow worker. Complete the assigned prompt and stop.",
          appendSystemPromptOverride: () => [],
        });
        await resourceLoader.reload();

        const { session } = await createAgentSession({
          cwd: ctx.cwd,
          agentDir,
          sessionManager: SessionManager.create(ctx.cwd),
          settingsManager,
          modelRegistry: ctx.modelRegistry,
          model,
          tools: [...CAPABILITY_TOOLS[capabilityMode]],
          resourceLoader,
        });
        await session.bindExtensions({});

        const set = activeByRun.get(request.run_id) ?? new Set();
        set.add(session);
        activeByRun.set(request.run_id, set);

        let cancelled = false;
        try {
          await session.prompt(request.prompt);
        } catch (e) {
          cancelled = true;
          writeResponse(responsePath, {
            success: false,
            output: "",
            error: e instanceof Error ? e.message : String(e),
            cancelled: true,
            child_session_id: session.sessionId,
            total_tokens_used: 0,
            duration_ms: Date.now() - started,
            backgrounded: false,
          });
          return;
        } finally {
          set.delete(session);
          if (set.size === 0) activeByRun.delete(request.run_id);
        }

        const output = lastAssistantText(session);
        writeResponse(responsePath, {
          success: true,
          output,
          cancelled: false,
          child_session_id: session.sessionId,
          total_tokens_used: 0,
          duration_ms: Date.now() - started,
          backgrounded: false,
        });

        // Notify parent bridge (appendEntry — never sendMessage during parent stream).
        pi.appendEntry(BRIDGE_TYPE, {
          version: 1,
          kind: "agent_finished",
          runId: request.run_id,
          agentId: id,
          childSessionId: session.sessionId,
          success: true,
        });
      } catch (e) {
        writeResponse(responsePath, {
          success: false,
          output: "",
          error: e instanceof Error ? e.message : String(e),
          cancelled: false,
          child_session_id: "",
          total_tokens_used: 0,
          duration_ms: Date.now() - started,
          backgrounded: false,
        });
      }
    },
  });

  pi.registerCommand(CANCEL_COMMAND, {
    description: "Internal: cancel workflow child agents",
    hidden: true,
    handler: async (args) => {
      const parsed = parseArgs(args);
      const runId = parsed["run-id"] ?? parsed.run_id;
      if (!runId) return;
      const set = activeByRun.get(runId);
      if (!set) return;
      for (const session of set) {
        try {
          session.abort();
        } catch {
          /* ignore */
        }
      }
      activeByRun.delete(runId);
    },
  });

  // Model-facing entry: host owns Rhai; this tool waits for the terminal outcome
  // via a response file so the parent turn gets the real report (not fire-and-forget).
  pi.registerTool({
    name: "workflow",
    label: "Workflow",
    description:
      "Launch or manage an upstream-compatible Rhai workflow (host-owned). Prefer named project workflows under .grok-pi/workflows (or $GROK_PROJECT_DIR). Blocks until the run finishes and returns the report.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "Workflow name or op (pause|resume|stop)" },
        args: { type: "string", description: "JSON args or objective text" },
      },
      required: ["name"],
    } as never,
    async execute(_toolCallId, params, signal, _onUpdate, ctx: ExtensionContext) {
      const name = String((params as { name?: string }).name ?? "");
      const args = String((params as { args?: string }).args ?? "");
      const { join } = await import("node:path");
      const { tmpdir } = await import("node:os");
      const { existsSync, readFileSync, mkdirSync } = await import("node:fs");
      const dir = join(tmpdir(), "pi-grok-workflow-tool");
      try {
        mkdirSync(dir, { recursive: true });
      } catch {
        /* ignore */
      }
      const responsePath = join(dir, `tool-${randomUUID()}.resp.json`);

      pi.appendEntry(BRIDGE_TYPE, {
        version: 1,
        kind: "tool_request",
        name,
        args,
        responsePath,
        cwd: ctx.cwd,
        parentSessionId: ctx.sessionManager.getSessionId(),
      });

      const started = Date.now();
      const maxMs = 4 * 60 * 60 * 1000; // 4h — deep-research can be long
      while (!existsSync(responsePath)) {
        if (signal?.aborted) {
          return {
            content: [
              {
                type: "text",
                text: `Workflow \`${name}\` aborted while waiting for host outcome.`,
              },
            ],
            details: { name, args, aborted: true },
          };
        }
        if (Date.now() - started > maxMs) {
          return {
            content: [
              {
                type: "text",
                text: `Workflow \`${name}\` timed out waiting for host outcome after ${maxMs}ms. Check /workflows or run status.`,
              },
            ],
            details: { name, args, timedOut: true },
          };
        }
        await new Promise((r) => setTimeout(r, 250));
      }

      let body: {
        text?: string;
        error?: string;
        runId?: string;
        outcome?: unknown;
        op?: string;
        ok?: boolean;
      } = {};
      try {
        body = JSON.parse(readFileSync(responsePath, "utf8")) as typeof body;
      } catch (e) {
        return {
          content: [
            {
              type: "text",
              text: `Workflow host response unreadable: ${e instanceof Error ? e.message : String(e)}`,
            },
          ],
          details: { name, args, responsePath },
        };
      }

      const text =
        body.text ??
        (body.error
          ? `Workflow failed: ${body.error}`
          : body.op
            ? `Workflow ${body.op}: ${JSON.stringify(body)}`
            : JSON.stringify(body, null, 2));

      return {
        content: [{ type: "text", text }],
        details: { name, args, runId: body.runId, outcome: body.outcome, responsePath },
      };
    },
  });
}
