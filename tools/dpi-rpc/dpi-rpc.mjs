#!/usr/bin/env node
// dpi-rpc: speaks `pi --mode rpc` (JSONL on stdio) and forwards to one
// dpi-backend /v1 agent, so grok-pi can attach to a durable pi-durable agent.
//
//   PI_BIN=tools/dpi-rpc/dpi-rpc.mjs ./run-local.sh <dir>
//
// Env: DPI_URL (default http://127.0.0.1:4780), DPI_TOKEN, DPI_SESSION (name or
// id; default: first session), DPI_AGENT (default main). Without env, DPI_URL/
// DPI_TOKEN come from ~/.dpi/backend.env (local backend), else ~/.dpi/client.env.
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

function fileEnv(name) {
  try {
    const out = {};
    for (const line of readFileSync(join(homedir(), ".dpi", name), "utf8").split("\n")) {
      const m = line.match(/^\s*([A-Z_]+)=(.*)$/);
      if (m) out[m[1]] = m[2].replace(/^["']|["']$/g, "");
    }
    return out;
  } catch {
    return {};
  }
}
const backendEnv = fileEnv("backend.env");
const clientEnv = fileEnv("client.env");
// A local backend (backend.env) wins over client.env, which points at a remote device.
const local = backendEnv.DPI_TOKEN ? { DPI_URL: `http://127.0.0.1:${backendEnv.DPI_PORT || 4780}`, DPI_TOKEN: backendEnv.DPI_TOKEN } : {};
const pick = (k) => process.env[k] || local[k] || clientEnv[k] || undefined;
const BASE = (pick("DPI_URL") || "http://127.0.0.1:4780").replace(/\/$/, "");
const TOKEN = pick("DPI_TOKEN");

let session = process.env.DPI_SESSION || null;
let agent = process.env.DPI_AGENT || "main";
let running = false;
let device = null;
let stream = null;
const toolOutput = new Map();

const out = (obj) => process.stdout.write(JSON.stringify(obj) + "\n");
const log = (...a) => process.stderr.write(`[dpi-rpc] ${a.join(" ")}\n`);

async function api(method, path, body) {
  const res = await fetch(BASE + "/v1" + path, {
    method,
    headers: {
      authorization: `Bearer ${TOKEN}`,
      ...(body === undefined ? {} : { "content-type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  const json = text ? JSON.parse(text) : null;
  if (!res.ok) throw new Error(json?.error?.message || `${method} ${path}: HTTP ${res.status}`);
  return json;
}
const agentPath = () => `/sessions/${encodeURIComponent(session)}/agents/${encodeURIComponent(agent)}`;

// ---- events: pi-durable watchEvents() -> Pi RPC events -------------------

function lastModel(entry) {
  const m = entry?.model;
  return Array.isArray(m) && m.length ? m[m.length - 1] : undefined;
}

function translate(e) {
  switch (e.type) {
    case "run_start":
      running = true;
      return [{ type: "agent_start" }];
    case "run_end":
      running = false;
      // Pi's idle barrier is agent_settled; the adapter completes prompts on it.
      return [{ type: "agent_end", messages: [] }, { type: "agent_settled" }];
    case "turn_start":
    case "turn_end":
      return [{ type: e.type }];
    case "message_start":
      return [{ type: "message_start", message: e.message }];
    case "message_end": {
      const message = lastModel(e.entry);
      return message ? [{ type: "message_end", message }] : [];
    }
    case "text_delta":
    case "thinking_delta":
      return [{ type: "message_update", assistantMessageEvent: { type: e.type, contentIndex: e.contentIndex, delta: e.delta } }];
    case "tool_execution_start":
      return [{ type: e.type, toolCallId: e.toolCallId, toolName: e.toolName, args: e.args }];
    case "tool_execution_update": {
      let buf = toolOutput.get(e.toolCallId) ?? "";
      if (e.output && "set" in e.output) buf = e.output.set;
      else if (e.output) buf = buf.slice(e.output.trimStart ?? 0) + (e.output.append ?? "");
      toolOutput.set(e.toolCallId, buf);
      return [{
        type: e.type, toolCallId: e.toolCallId, toolName: e.toolName,
        partialResult: { content: [{ type: "text", text: buf }], details: e.details },
      }];
    }
    case "tool_execution_end": {
      toolOutput.delete(e.toolCallId);
      const msg = lastModel(e.entry);
      return [{
        type: e.type, toolCallId: e.toolCallId, toolName: e.toolName,
        result: { content: msg?.content ?? [{ type: "text", text: "(tool task faulted or was orphaned)" }], details: e.entry?.data },
        isError: msg ? !!msg.isError : true,
      }];
    }
    case "auto_retry_start":
      return [{ type: e.type, attempt: e.attempt, errorMessage: e.errorMessage }];
    case "auto_retry_end":
      return [{ type: e.type, success: true }];
    default:
      return [];
  }
}

async function runStream(signal) {
  for (let backoff = 500; !signal.aborted; backoff = Math.min(backoff * 2, 10000)) {
    try {
      const res = await fetch(BASE + "/v1" + agentPath() + "/events", {
        headers: { authorization: `Bearer ${TOKEN}`, accept: "text/event-stream" },
        signal,
      });
      if (!res.ok) throw new Error(`events: HTTP ${res.status}`);
      backoff = 500;
      const decoder = new TextDecoder();
      let buf = "";
      for await (const chunk of res.body) {
        buf += decoder.decode(chunk, { stream: true });
        let i;
        while ((i = buf.indexOf("\n\n")) >= 0) {
          const frame = buf.slice(0, i);
          buf = buf.slice(i + 2);
          const data = frame.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).trimStart()).join("\n");
          if (!data) continue;
          const msg = JSON.parse(data);
          if (msg.type === "batch") for (const e of msg.events) for (const t of translate(e)) out(t);
        }
      }
    } catch (err) {
      if (signal.aborted) return;
      log(`stream: ${err.message}; reconnecting`);
    }
    await new Promise((r) => setTimeout(r, backoff));
  }
}

// `--continue` means the durable agent this machine last attached to.
const LAST = join(homedir(), ".dpi", "grok-pi-last.json");

function argValue(flag) {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

// dpi://<session>/<agent> | <session>:<agent> | <session name or id>
function parseTarget(value) {
  const url = value.match(/^dpi:\/\/([^/]+)(?:\/([^/]+))?/);
  if (url) return [decodeURIComponent(url[1]), url[2] ? decodeURIComponent(url[2]) : "main"];
  const pair = value.match(/^([0-9a-f-]{36}):(.+)$/);
  if (pair) return [pair[1], pair[2]];
  return [value, "main"];
}

async function pickTarget() {
  const explicit = argValue("--session");
  if (explicit) return parseTarget(explicit);
  if (session) return [session, agent];
  const sessions = await api("GET", "/sessions");
  if (!sessions.length) throw new Error("no dpi sessions; create one or set DPI_SESSION");
  try {
    const last = JSON.parse(readFileSync(LAST, "utf8"));
    if (sessions.some((s) => s.id === last.session)) return [last.session, last.agent];
  } catch {}
  const newest = sessions.reduce((a, b) => (b.createdAt > a.createdAt ? b : a));
  return [newest.id, "main"];
}

function attach() {
  try { writeFileSync(LAST, JSON.stringify({ session, agent })); } catch {}
  stream?.abort();
  stream = new AbortController();
  running = false;
  runStream(stream.signal);
}

// ---- commands: Pi RPC -> /v1 ---------------------------------------------

async function activeMessages() {
  const { items } = await api("GET", agentPath() + "/entries?scope=active");
  return items.flatMap((i) => i.model ?? []).filter((m) => m.role !== "system");
}

async function getState() {
  const [a, s] = await Promise.all([api("GET", agentPath()), api("GET", `/sessions/${encodeURIComponent(session)}`)]);
  const { provider, modelId } = a.config.model;
  const known = device.models.find((m) => m.provider === provider && m.id === modelId);
  return {
    sessionId: `${s.id}:${a.id}`,
    sessionName: a.kind === "root" ? s.name : `${s.name} / ${a.name}`,
    model: { ...(known ?? {}), provider, id: modelId },
    thinkingLevel: a.config.thinkingLevel ?? "medium",
    isStreaming: running || !!a.busy,
    isCompacting: false,
    pendingMessageCount: 0,
  };
}

async function sessionStats() {
  const messages = await activeMessages();
  const lastAssistant = [...messages].reverse().find((m) => m.role === "assistant" && m.usage);
  const u = lastAssistant?.usage;
  const tokens = u ? (u.input ?? 0) + (u.output ?? 0) + (u.cacheRead ?? 0) + (u.cacheWrite ?? 0) : 0;
  const a = await api("GET", agentPath());
  const known = device.models.find((m) => m.provider === a.config.model.provider && m.id === a.config.model.modelId);
  return {
    sessionId: session,
    totalMessages: messages.length,
    userMessages: messages.filter((m) => m.role === "user").length,
    assistantMessages: messages.filter((m) => m.role === "assistant").length,
    contextUsage: { tokens, contextWindow: known?.contextWindow ?? 200000 },
  };
}

async function send(content, whenBusy) {
  if (typeof content !== "string") content = JSON.stringify(content);
  await api("POST", agentPath() + "/messages", { content, whenBusy, requestId: randomUUID() });
}

const handlers = {
  get_state: getState,
  // Re-read so model list edits on the backend show without restarting grok-pi.
  get_available_models: async () => ((device = await api("GET", "/device")), { models: device.models }),
  get_commands: async () => ({ commands: [] }),
  get_messages: async () => ({ messages: await activeMessages() }),
  get_entries: async () => {
    const msgs = await activeMessages();
    const entries = msgs.map((message, i) => ({
      type: "message", id: `e${i}`, parentId: i ? `e${i - 1}` : null,
      timestamp: new Date(message.timestamp ?? Date.now()).toISOString(), message,
    }));
    return { entries, leafId: entries.at(-1)?.id ?? null };
  },
  get_session_stats: sessionStats,
  // The durable agent is the session; a fresh TUI attaches instead of resetting it.
  new_session: async () => ({ cancelled: false }),
  prompt: async (c) => send(c.message, c.streamingBehavior === "steer" ? "steer" : "followUp"),
  steer: async (c) => send(c.message, "steer"),
  follow_up: async (c) => send(c.message, "followUp"),
  abort: async () => { await api("POST", agentPath() + "/abort", {}); },
  set_model: async (c) => {
    await api("PATCH", agentPath() + "/config", { model: { provider: c.provider, modelId: c.modelId } });
    return device.models.find((m) => m.provider === c.provider && m.id === c.modelId) ?? { provider: c.provider, id: c.modelId };
  },
  set_thinking_level: async (c) => { await api("PATCH", agentPath() + "/config", { thinkingLevel: c.level }); },
  compact: async (c) => api("POST", agentPath() + "/compact", c.customInstructions ? { instructions: c.customInstructions } : {}),
  set_session_name: async (c) => { await api("PATCH", `/sessions/${encodeURIComponent(session)}`, { name: c.name }); },
  set_steering_mode: async () => {},
  set_follow_up_mode: async () => {},
  set_auto_compaction: async () => {},
  set_auto_retry: async () => {},
  // grok-pi's /resume picker (GROK_PI_DPI=1): one row per durable agent.
  dpi_list_sessions: async () => {
    const sessions = await api("GET", "/sessions");
    return {
      sessions: sessions.flatMap((s) => (s.agents ?? []).map((a) => ({
        id: `${s.id}:${a.id}`,
        path: `dpi://${s.id}/${a.id}`,
        name: a.kind === "root" ? s.name : `${s.name} / ${a.name}`,
        cwd: s.cwd,
        createdAt: new Date(s.createdAt).toISOString(),
        modifiedAt: new Date(s.createdAt).toISOString(),
        firstMessage: a.busy ? "(running)" : "",
        modelId: s.model,
        parentSessionPath: a.kind === "root" ? undefined : `dpi://${s.id}/main`,
      }))),
    };
  },
  // sessionPath: dpi://<session>[/<agent>]
  switch_session: async (c) => {
    if (!String(c.sessionPath ?? "").startsWith("dpi://")) throw new Error(`not a dpi session path: ${c.sessionPath}`);
    [session, agent] = parseTarget(String(c.sessionPath));
    attach();
    return { cancelled: false };
  },
};

async function handle(cmd) {
  const fn = handlers[cmd.type];
  try {
    if (!fn) throw new Error(`${cmd.type} is not supported over dpi`);
    const data = await fn(cmd);
    if (cmd.id !== undefined) out({ type: "response", id: cmd.id, command: cmd.type, success: true, ...(data === undefined ? {} : { data }) });
  } catch (err) {
    if (cmd.id !== undefined) out({ type: "response", id: cmd.id, command: cmd.type, success: false, error: err.message });
    else log(`${cmd.type}: ${err.message}`);
  }
}

async function main() {
  // grok-pi probes `--version` (min 0.99.0) before spawning RPC mode.
  if (process.argv.includes("--version")) return console.log("dpi-rpc 0.99.0");
  if (!TOKEN) throw new Error("DPI_TOKEN not set (env, ~/.dpi/client.env or ~/.dpi/backend.env)");
  device = await api("GET", "/device");
  [session, agent] = await pickTarget();
  // Resolve names to ids so the remembered target survives renames.
  const a = await api("GET", agentPath());
  session = a.sessionId;
  agent = String(a.id);
  attach();
  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    if (!line.trim()) return;
    let cmd;
    try { cmd = JSON.parse(line); } catch { return log(`bad json: ${line.slice(0, 200)}`); }
    handle(cmd);
  });
  rl.on("close", () => process.exit(0));
}

main().catch((err) => { log(err.message); process.exit(1); });
