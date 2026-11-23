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
import { basename, join } from "node:path";

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
let awaitingRun = false; // a prompt was sent and its run_start has not arrived
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
  if (!res.ok) throw Object.assign(new Error(json?.error?.message || `${method} ${path}: HTTP ${res.status}`), { status: res.status });
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
      awaitingRun = false;
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

// The backend writes ": ping" every 15s. A laptop that slept keeps a dead socket
// that never errors, so silence past this limit means reconnect.
const STREAM_SILENCE_MS = 45_000;

const maxEntryId = (entries, floor) =>
  (entries ?? []).reduce((m, x) => (Number.isInteger(x?.id) && x.id > m ? x.id : m), floor);

// After a reconnect: replay the entries this TUI missed, and end a turn that
// finished while the stream was down so the TUI does not wait on it forever.
async function catchUp(snapshot, lastSeen) {
  const missed = (snapshot?.entries ?? []).filter((x) => Number.isInteger(x?.id) && x.id > lastSeen);
  // A prompt this TUI sent ran entirely while the stream was down: open the run so it is acknowledged.
  if (awaitingRun && missed.length) for (const t of translate({ type: "run_start" })) out(t);
  for (const entry of missed) for (const t of translate({ type: "message_end", entry })) out(t);
  if (!running) return;
  try {
    const a = await api("GET", agentPath());
    if (!a.busy) for (const t of translate({ type: "run_end" })) out(t);
  } catch (err) {
    log(`catch-up: ${err.message}`);
  }
}

async function runStream(signal) {
  let lastSeen = null; // highest entry id delivered; null until the first snapshot
  for (let backoff = 500; !signal.aborted; backoff = Math.min(backoff * 2, 10000)) {
    const conn = new AbortController();
    const onAbort = () => conn.abort();
    signal.addEventListener("abort", onAbort, { once: true });
    let heard = Date.now();
    const watchdog = setInterval(() => {
      if (Date.now() - heard > STREAM_SILENCE_MS) conn.abort(new Error(`no data for ${STREAM_SILENCE_MS / 1000}s`));
    }, 5_000);
    try {
      const res = await fetch(BASE + "/v1" + agentPath() + "/events", {
        headers: { authorization: `Bearer ${TOKEN}`, accept: "text/event-stream" },
        signal: conn.signal,
      });
      if (!res.ok) throw new Error(`events: HTTP ${res.status}`);
      backoff = 500;
      const decoder = new TextDecoder();
      let buf = "";
      for await (const chunk of res.body) {
        heard = Date.now();
        buf += decoder.decode(chunk, { stream: true });
        let i;
        while ((i = buf.indexOf("\n\n")) >= 0) {
          const frame = buf.slice(0, i);
          buf = buf.slice(i + 2);
          const data = frame.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).trimStart()).join("\n");
          if (!data) continue;
          const msg = JSON.parse(data);
          if (msg.type === "snapshot") {
            if (lastSeen !== null) await catchUp(msg.snapshot, lastSeen);
            lastSeen = maxEntryId(msg.snapshot?.entries, lastSeen ?? 0);
          } else if (msg.type === "batch") {
            for (const e of msg.events) {
              for (const t of translate(e)) out(t);
              if (Number.isInteger(e.entry?.id)) lastSeen = Math.max(lastSeen ?? 0, e.entry.id);
            }
          }
        }
      }
      throw new Error("stream ended");
    } catch (err) {
      if (signal.aborted) return;
      log(`stream: ${conn.signal.reason?.message ?? err.message}; reconnecting`);
    } finally {
      clearInterval(watchdog);
      signal.removeEventListener("abort", onAbort);
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
  awaitingRun = false;
  runStream(stream.signal);
  watchTasks(stream.signal);
}

// ---- commands: Pi RPC -> /v1 ---------------------------------------------

const activeEntries = async (path = agentPath()) => (await api("GET", path + "/entries?scope=active")).items;
const modelMessages = (entries) => entries.flatMap((i) => i.model ?? []).filter((m) => m.role !== "system");
const activeMessages = async () => modelMessages(await activeEntries());

// ---- live tasks (and Prime's goal/heartbeat) as one status line each ----------

const TASK_POLL_MS = Number(process.env.DPI_TASK_POLL_MS) || 3000;
const clip = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
let primeWork = true; // false once a backend answers 404 for prime/work: only prime-backend serves it

const taskText = (tasks) => {
  // A foreground tool call already shows as a tool event; only background and job tasks are news.
  const live = tasks.filter((t) => t.background || t.kind !== "pi.tool");
  return live.length ? `${live.length} task${live.length > 1 ? "s" : ""}: ${live.slice(0, 3).map((t) => t.label ?? t.kind).join(", ")}` : undefined;
};

const primeText = (w) => {
  const parts = [];
  if (w.goal) parts.push(`goal ${w.goal.status}: ${clip(w.goal.objective, 60)}`);
  if (w.heartbeat) parts.push(`heartbeat ${w.heartbeat.status} ${w.heartbeat.schedule}`);
  return parts.length ? parts.join(" | ") : undefined;
};

function watchTasks(signal) {
  const shown = new Map(); // statusKey -> last text, so only changes reach the TUI
  const show = (statusKey, statusText) => {
    if (shown.get(statusKey) === statusText) return;
    shown.set(statusKey, statusText);
    out({ type: "extension_ui_request", id: randomUUID(), method: "setStatus", statusKey, statusText });
  };
  watchDeploys(signal, show);
  (async () => {
    while (!signal.aborted) {
      const base = `/sessions/${encodeURIComponent(session)}`;
      try { show("dpi-tasks", taskText(await api("GET", base + "/tasks?status=live"))); } catch (err) { log(`tasks: ${err.message}`); }
      if (primeWork) {
        try { show("prime-work", primeText(await api("GET", base + "/prime/work"))); } catch (err) { if (err.status === 404) primeWork = false; else log(`prime work: ${err.message}`); }
      }
      await new Promise((r) => setTimeout(r, TASK_POLL_MS));
    }
  })();
}

// ---- proactive-demo deploy approvals: ops service -> confirm dialog -> decide ------
// Env: DPI_OPS_URL or OPS_URL (default http://127.0.0.1:4790), DPI_OPS_POLL_MS (default 3000),
// DPI_OPS_REPROMPT_MS (default 600000: how long a dismissed prompt stays quiet),
// DPI_OPS_DIALOG_MS (default 300000: a dialog retracts itself after this and is shown again if still pending).
// An unreachable ops service is silent: most sessions never have one.

const OPS_URL = (process.env.DPI_OPS_URL || process.env.OPS_URL || "http://127.0.0.1:4790").replace(/\/$/, "");
const OPS_POLL_MS = Number(process.env.DPI_OPS_POLL_MS) || 3000;
const OPS_REPROMPT_MS = Number(process.env.DPI_OPS_REPROMPT_MS) || 600_000;
const asked = new Set(); // deploy keys already prompted: one dialog per request, across re-attaches
const OPS_DIALOG_MS = Number(process.env.DPI_OPS_DIALOG_MS) || 300_000;
const dialogs = new Map(); // confirm request id -> {deployId, key, at, gone?}; gone = {how, at} once decided elsewhere
let awaiting = new Set(); // deploy keys awaiting approval at the last poll
// An id alone is not unique: a demo reset restarts ids, so a request is its id plus when it was made.
const deployKey = (d) => `${d.id}:${d.requestedAt}`;

async function ops(method, path, body) {
  const res = await fetch(OPS_URL + "/api" + path, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(5000),
  });
  if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status}`);
  return res.json();
}

const notify = (message) => out({ type: "extension_ui_request", id: randomUUID(), method: "notify", message, notifyType: "info" });
const hhmm = (ms) => new Date(ms).toLocaleTimeString("sv").slice(0, 5);
// How a deploy left awaiting_approval, from its row: "approved", "declined", ... and the local HH:MM it was decided.
const outcome = (d) => ({ how: { declined: "declined", cancelled: "cancelled" }[d?.status] ?? (d ? "approved" : "resolved"), at: d?.decidedAt ? ` at ${hhmm(d.decidedAt)}` : "" });

function watchDeploys(signal, show) {
  (async () => {
    while (!signal.aborted) {
      try {
        const deploys = await ops("GET", "/deploys");
        awaiting = new Set(deploys.filter((d) => d.status === "awaiting_approval").map(deployKey));
        for (const d of deploys) {
          const key = deployKey(d);
          if (!awaiting.has(key) || asked.has(key)) continue;
          asked.add(key);
          const id = randomUUID();
          dialogs.set(id, { deployId: d.id, key, at: Date.now() });
          // The adapter shows `message` (not `title`) as a confirm's question, so the title leads it.
          const title = `Deploy ${d.incidentId}: ${d.summary}`;
          const message = [title, `commit ${d.commit}`, `branch ${d.branch}`, d.diffStat, "If this was already decided elsewhere (hub, another device) you can dismiss this; dismissing records nothing."].join("\n");
          // timeout: the adapter retracts the dialog itself after it, the only retraction it offers.
          out({ type: "extension_ui_request", id, method: "confirm", title, message, timeout: OPS_DIALOG_MS });
        }
        for (const dlg of dialogs.values()) {
          if (awaiting.has(dlg.key) || dlg.gone) continue;
          // Decided elsewhere. The adapter cannot retract an open dialog early: caption it, ignore its answer.
          dlg.gone = outcome(deploys.find((d) => deployKey(d) === dlg.key));
          notify(`Deploy ${dlg.deployId} was ${dlg.gone.how} elsewhere${dlg.gone.at}. You can dismiss the open dialog; nothing was recorded from it.`);
        }
        show("ops-deploy", awaiting.size ? `${awaiting.size} deploy${awaiting.size > 1 ? "s" : ""} awaiting approval` : undefined);
      } catch {}
      await new Promise((r) => setTimeout(r, OPS_POLL_MS));
    }
  })();
}

// The TUI's answer to a confirm. Only an explicit confirmed true/false is a decision; a dismissed
// dialog records nothing and the deploy is offered again after the cooldown.
function onUiResponse(r) {
  const dlg = dialogs.get(r.id);
  if (dlg === undefined) return;
  dialogs.delete(r.id);
  const { deployId } = dlg;
  if (typeof r.confirmed === "boolean" && dlg.gone) return notify(`Deploy ${deployId} was already ${dlg.gone.how}${dlg.gone.at}; your ${r.confirmed ? "Yes" : "No"} was not sent.`);
  if (dlg.gone) return; // timed out or dismissed after it went stale: nothing to say
  if (typeof r.confirmed !== "boolean") {
    if (Date.now() - dlg.at >= OPS_DIALOG_MS) return asked.delete(dlg.key); // the adapter's timeout retracted it: show it again
    notify(`No decision recorded for deploy ${deployId}; it is still awaiting approval`);
    setTimeout(() => asked.delete(dlg.key), OPS_REPROMPT_MS).unref();
    return;
  }
  // Only a decide that returned ok is reported as recorded; the answer is the user's, so the wording follows it.
  ops("POST", `/deploys/${deployId}/decide`, { approved: r.confirmed }).then(
    () => notify(`Deploy ${deployId} ${r.confirmed ? "approved" : "declined"}.`),
    (err) => { log(`deploy ${deployId}: ${err.message}`); notify(`Deploy ${deployId}: could not record your answer: ${err.message}`); },
  );
}

// Pi's /fork picker rows: one per user turn, keyed by the dpi entry id.
const textOf = (content) => (typeof content === "string" ? content : (content ?? []).filter((p) => p.type === "text").map((p) => p.text).join(""));
const forkMessages = (items) => items
  .filter((e) => e.kind === "pi.user")
  .map((e) => ({ entryId: String(e.id), text: textOf(e.model?.[0]?.content) }));

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

// The system prompt and tools the model is sent, replayed from the transcript's system messages the way
// pi-ai's getCurrentSystemMessage does: content appends, sections patch by name (null removes), tools by name.
function systemContext(entries) {
  const content = [];
  const sections = new Map();
  const tools = new Map();
  for (const m of entries.flatMap((e) => e.model ?? []).filter((m) => m.role === "system")) {
    const text = textOf(m.content);
    if (text) content.push(text);
    for (const [k, v] of Object.entries(m.sections ?? {})) v === null ? sections.delete(k) : sections.set(k, v);
    for (const t of m.toolsRemoved ?? []) tools.delete(t.name);
    for (const t of m.toolsAdded ?? []) tools.set(t.name, t);
  }
  return { prompt: [content.join("\n\n"), ...sections.values()].filter(Boolean).join("\n\n"), tools: [...tools.values()] };
}

// grok-pi's /context asks for this (pi-grok-context's payload): raw ceil(len/4) estimates it scales to the real total.
const CONTEXT_BREAKDOWN = "__pi_context_breakdown";
const estimate = (text) => Math.ceil((text?.length ?? 0) / 4);
async function writeContextBreakdown() {
  const outPath = process.env.PI_GROK_CONTEXT_BREAKDOWN;
  if (!outPath) throw new Error("PI_GROK_CONTEXT_BREAKDOWN is not set");
  const { prompt, tools } = systemContext(await activeEntries());
  const defs = tools.map(({ name, description, parameters }) => ({ name, description, parameters }));
  writeFileSync(outPath, JSON.stringify({
    version: 1,
    systemPromptTokensRaw: estimate(prompt),
    toolDefinitionsCount: defs.length,
    toolDefinitionsTokensRaw: defs.length ? estimate(JSON.stringify(defs)) : 0,
    appendTokensRaw: 0,
    contextFiles: [],
    skillsCount: 0,
    skillsTokensRaw: 0,
  }));
}

async function send(content, whenBusy) {
  if (typeof content !== "string") content = JSON.stringify(content);
  await api("POST", agentPath() + "/messages", { content, whenBusy, requestId: randomUUID() });
  if (!running) awaitingRun = true;
}

// Slash commands the shim answers itself. Pi RPC delivers them as `prompt` "/name args"; source "extension"
// makes grok-pi run them at once without a turn, and they never reach the agent's transcript.
async function shimCommands() {
  return new Map([
    [CONTEXT_BREAKDOWN, { description: "Internal Pi-Grok bridge: write context token breakdown", run: writeContextBreakdown }],
  ]);
}

// true when `message` was one of the shim's commands (and it ran), false for anything to send to the agent.
async function runCommand(message) {
  const m = typeof message === "string" && message.match(/^\/(\S+)\s*([\s\S]*)$/);
  if (!m) return false;
  const cmd = (await shimCommands()).get(m[1]);
  if (!cmd) return false;
  await cmd.run(m[2].trim());
  return true;
}

const handlers = {
  get_state: getState,
  // Re-read so model list edits on the backend show without restarting grok-pi.
  get_available_models: async () => ((device = await api("GET", "/device")), { models: device.models }),
  get_commands: async () => ({ commands: [...(await shimCommands())].map(([name, c]) => ({ name, description: c.description, source: "extension" })) }),
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
  get_fork_messages: async () => ({ messages: forkMessages(await activeEntries()) }),
  // Like Pi, fork before the picked user turn and hand its text back for the editor, then follow the fork.
  // dpi keeps the entry it forks at, so fork at the one before; the first turn has none: start a fresh agent.
  fork: async (c) => {
    const items = await activeEntries();
    const text = forkMessages(items).find((m) => m.entryId === String(c.entryId))?.text;
    if (text === undefined) throw new Error(`entryId is not a user turn of this agent: ${c.entryId}`);
    const before = items[items.findIndex((e) => String(e.id) === String(c.entryId)) - 1];
    const forked = before
      ? await api("POST", agentPath() + "/fork", { entryId: before.id })
      : await api("POST", `/sessions/${encodeURIComponent(session)}/agents`, { from: agent });
    agent = String(forked.id);
    attach();
    return { cancelled: false, text };
  },
  // A new durable session in the current one's cwd with its model, then attach to it.
  new_session: async () => {
    const a = await api("GET", agentPath());
    const cwd = a.config.cwd ?? (await api("GET", `/sessions/${encodeURIComponent(session)}`)).cwd;
    const stamp = new Date().toLocaleString("sv").slice(5, 16);
    const created = await api("POST", "/sessions", { name: `${basename(cwd)} ${stamp}`, cwd, model: a.config.model, thinkingLevel: a.config.thinkingLevel });
    [session, agent] = [created.id, "main"];
    attach();
    return { cancelled: false };
  },
  prompt: async (c) => { if (!(await runCommand(c.message))) await send(c.message, c.streamingBehavior === "steer" ? "steer" : "followUp"); },
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
  // The tree has no activity time or counts, so each row reads its agent's active transcript.
  // shortcut: one fetch per agent on every picker open; add a backend field or cache if sessions grow.
  dpi_list_sessions: async () => {
    const rows = (await api("GET", "/sessions")).flatMap((s) => (s.agents ?? []).map((a) => [s, a]));
    const active = await Promise.all(rows.map(([s, a]) => activeEntries(`/sessions/${s.id}/agents/${a.id}`).catch(() => [])));
    return {
      sessions: rows.map(([s, a], i) => {
        const messages = modelMessages(active[i]);
        const last = Math.max(s.createdAt, ...messages.map((m) => m.timestamp ?? 0));
        return {
          id: `${s.id}:${a.id}`,
          path: `dpi://${s.id}/${a.id}`,
          name: a.kind === "root" ? s.name : `${s.name} / ${a.name}`,
          cwd: s.cwd,
          createdAt: new Date(s.createdAt).toISOString(),
          modifiedAt: new Date(last).toISOString(),
          messageCount: messages.length,
          firstMessage: a.busy ? "(running)" : "",
          modelId: s.model,
          parentSessionPath: a.kind === "root" ? undefined : `dpi://${s.id}/${a.parentId ?? a.ownerAgentId ?? "main"}`,
        };
      }),
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
  let a;
  try { a = await api("GET", agentPath()); } catch (err) {
    // The pager buries this line under its own bootstrap text, so say the cause plainly.
    throw err.status === 404 ? new Error(`no dpi session named ${argValue("--session") ?? session} on ${BASE}`) : err;
  }
  session = a.sessionId;
  agent = String(a.id);
  if (process.argv.includes("--resolve")) return console.log(`${session}:${agent}`); // for dpi-grok's pre-check and --resume
  attach();
  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    if (!line.trim()) return;
    let cmd;
    try { cmd = JSON.parse(line); } catch { return log(`bad json: ${line.slice(0, 200)}`); }
    if (cmd.type === "extension_ui_response") return onUiResponse(cmd);
    handle(cmd);
  });
  rl.on("close", () => process.exit(0));
}

main().catch((err) => { log(err.message); process.exit(1); });
