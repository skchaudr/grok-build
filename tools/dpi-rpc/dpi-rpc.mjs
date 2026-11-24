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
import {
  THINKING_LEVELS, buildOptions, commandForKey, compactionPatch, contextInfo, helpCard, hintsFor,
  resolvePlaygroundUrl, shortcutCatalog, usageTokens,
} from "./settings.mjs";

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
              if (e.type === "run_end") refreshRemote(); // a turn may have switched branches
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

// Leaving an agent mid-turn (dashboard dispatch, resume, fork): end the turn for the TUI so its prompt does not wait
// forever. Nothing is sent to the backend: the run carries on there and its row shows it working.
function releaseTurn() {
  if (awaitingRun && !running) for (const t of translate({ type: "run_start" })) out(t);
  if (running) for (const t of translate({ type: "run_end" })) out(t);
}

function attach() {
  releaseTurn();
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
// dpi_list_sessions' transcript per agent. Re-read while the agent is busy, when it was busy at the last read, or after
// LISTED_TTL_MS; otherwise an idle agent's transcript has not changed since.
const LISTED_TTL_MS = 30_000;
const listed = new Map(); // "<session>:<agent>" -> { busy, at, entries }
async function listedEntries(s, a) {
  const key = `${s.id}:${a.id}`;
  const hit = listed.get(key);
  if (hit && !a.busy && !hit.busy && Date.now() - hit.at < LISTED_TTL_MS) return hit.entries;
  const entries = await activeEntries(`/sessions/${s.id}/agents/${a.id}`).catch(() => null);
  if (entries === null) { listed.delete(key); return []; }
  listed.set(key, { busy: !!a.busy, at: Date.now(), entries });
  return entries;
}
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
const dialogs = new Map(); // deploy confirm request id -> {deployId, key, at, gone?}; gone = {how, at} once decided elsewhere
// Every dialog we opened and still await: extension_ui_request id -> fn(response). Deploy approvals and confirm() both live here.
const pending = new Map();
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

const notify = (message, notifyType = "info") => out({ type: "extension_ui_request", id: randomUUID(), method: "notify", message, notifyType });
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
          pending.set(id, (r) => answerDeploy(id, r));
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

// The TUI's answer to a dialog we opened; answers to ids we did not open are ignored.
function onUiResponse(r) {
  const answer = pending.get(r.id);
  if (!answer) return;
  pending.delete(r.id);
  answer(r);
}

// Ask a yes/no question and run `action` on Yes. Returns at once: the answer arrives later as an extension_ui_response,
// and holding the prompt open meanwhile would stall the adapter. A dismissed or timed-out dialog does nothing.
const CONFIRM_MS = Number(process.env.DPI_CONFIRM_MS) || 120_000;
function confirmThen(title, message, action) {
  const id = randomUUID();
  const timer = setTimeout(() => pending.delete(id), CONFIRM_MS + 5000).unref(); // the adapter retracts the dialog at CONFIRM_MS
  pending.set(id, (r) => {
    clearTimeout(timer);
    if (r.confirmed === true) action().catch((err) => notify(`${title}: ${err.message}`, "error"));
    else notify(`${title}: nothing done.`);
  });
  out({ type: "extension_ui_request", id, method: "confirm", title, message, timeout: CONFIRM_MS });
}

// A deploy dialog's answer. Only an explicit confirmed true/false is a decision; a dismissed
// dialog records nothing and the deploy is offered again after the cooldown.
function answerDeploy(id, r) {
  const dlg = dialogs.get(id);
  dialogs.delete(id);
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

// ---- where the agent really runs, for grok-pi's header and attach card (GROK_PI_DPI=1) ----

const tilde = (path) => path.replace(/^(?:\/Users\/[^/]+|\/home\/[^/]+|\/root)(?=\/|$)/, "~");
// The backend's device as people call it: `sab-mini`, not `sab-mini.local` or a URL.
const hostName = () => String(device.hostname || device.name || new URL(BASE).hostname).replace(/\.local$/, "");
const ago = (ms) => {
  const m = Math.round((Date.now() - ms) / 60000);
  return m < 1 ? "just now" : m < 60 ? `${m}m ago` : m < 2880 ? `${Math.round(m / 60)}h ago` : `${Math.round(m / 1440)}d ago`;
};

// {host, cwd, label: "host:~/path", branch?}; with `card`, also the attach card's title and rows.
async function remote(card = false) {
  const a = await api("GET", agentPath());
  const s = card || !a.config.cwd ? await api("GET", `/sessions/${encodeURIComponent(session)}`) : null;
  const host = hostName();
  const cwd = a.config.cwd ?? s.cwd;
  const r = { host, cwd, label: `${host}:${tilde(cwd)}`, ...(a.branch ? { branch: a.branch } : {}) };
  if (!card) return r;
  const [entries, live] = await Promise.all([activeEntries(), api("GET", `/sessions/${encodeURIComponent(session)}/tasks?status=live`).catch(() => [])]);
  const messages = modelMessages(entries);
  const lastUser = [...messages].reverse().find((m) => m.role === "user");
  const lastAt = Math.max(0, ...messages.map((m) => m.timestamp ?? 0));
  const activity = a.busy ? "running now" : lastAt ? `last message ${ago(lastAt)}` : "no messages yet";
  r.card = {
    title: `dpi · ${a.kind === "root" ? s.name : `${s.name} / ${a.name}`} on ${host}`,
    rows: [
      ["model", `${a.config.model.modelId} (${a.config.model.provider}) · thinking ${a.config.thinkingLevel ?? "medium"}`],
      ["cwd", `${r.label}${r.branch ? ` · ${r.branch}` : ""}`],
      ["activity", `${activity}${messages.length ? ` · ${messages.length} messages` : ""}`],
      ...(lastUser ? [["last ask", clip(textOf(lastUser.content).replace(/\s+/g, " ").trim(), 100)]] : []),
      ["tasks", taskText(live) ?? "none live"],
    ],
    playgroundUrl: await playgroundUrl(), // string, or null when this backend has no playground
  };
  return r;
}

// After a turn, tell the TUI if the agent's location changed (a checkout or cd on the backend).
let lastRemote = null; // the location the TUI last got, as JSON
const sentRemote = (r) => { const was = lastRemote; lastRemote = JSON.stringify({ ...r, card: undefined }); return lastRemote !== was; };
function refreshRemote() {
  remote().then((r) => { if (sentRemote(r)) out({ type: "extension_ui_request", id: randomUUID(), method: "setRemote", ...r }); }, (err) => log(`remote: ${err.message}`));
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
  const tokens = usageTokens(messages) ?? 0;
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
  const prompts = await savedPrompts().catch((err) => (log(`prompts: ${err.message}`), []));
  return new Map([
    [CONTEXT_BREAKDOWN, { description: "Internal Pi-Grok bridge: write context token breakdown", run: writeContextBreakdown }],
    [SHORTCUT_DISPATCH, { description: "Internal Pi-Grok bridge: run the Durable Pi action bound to a key", run: dispatchShortcut }],
    ["dpi-help", { description: "What Durable Pi is, its keys and commands", run: showHelp }],
    ["dpi-settings", { description: "Summarise this agent's settings (F2 opens them)", run: showSettings }],
    ["dpi-open", { description: "Print the playground URL", run: showPlayground }],
    ["dpi-compact", { description: "Summarise older context now; stored history stays", args: "[instructions]", run: compactNow }],
    ["dpi-reset", { description: "Start a fresh context after a confirm; history stays stored", args: "[handoff note]", run: resetAgent }],
    ["dpi-fork", { description: "Branch this agent at its last reply into a new agent", args: "[name]", run: forkAgent }],
    ["dpi-new-agent", { description: "Add a new agent to this session", args: "[name]", run: newAgent }],
    ["dpi-rename", { description: "Rename this agent (the session, for the main agent)", args: "<name>", run: renameAgent }],
    ["dpi-stop", { description: "Stop the running turn and its background tasks", run: stopAgent }],
    ["dpi-archive", { description: "Archive this session after a confirm; nothing is deleted", run: archiveSession }],
    ["dpi-sessions", { description: "List the sessions on this dpi backend (ctrl+r switches)", run: listSessions }],
    ["dpi-tasks", { description: "Show this session's live and recent tasks", run: listTasks }],
    ["dpi-deploys", { description: "Show deploys awaiting approval and reopen their dialogs", run: listDeploys }],
    ...prompts.map((p) => [`prompt-${p.id.replace(/\.md$/, "").replace(/[^a-z0-9]+/gi, "-").toLowerCase()}`, {
      description: `Turn the saved system prompt "${p.title}" on or off for this agent`,
      run: () => togglePrompt(p.id),
    }]),
  ]);
}

// The backend's saved system prompts: a prompt is in effect when its checkbox is on and the system-prompts extension is.
const SYSTEM_PROMPTS = "system-prompts";
const getExtensions = async () => (await api("GET", agentPath() + "/extensions")).extensions;
async function savedPrompts() {
  const ext = (await getExtensions()).find((e) => e.name === SYSTEM_PROMPTS);
  return (ext?.prompts ?? []).map((p) => ({ ...p, active: ext.enabled && p.enabled }));
}

// Turn one saved prompt on or off; turning one on also turns the extension on, or it would do nothing.
async function setPrompt(id, on) {
  const ext = (await getExtensions()).find((e) => e.name === SYSTEM_PROMPTS);
  const prompt = ext?.prompts?.find((p) => p.id === id);
  if (!prompt) throw new Error(`no saved system prompt ${id} on this backend`);
  if (prompt.enabled !== on) await api("PATCH", agentPath() + "/extensions", { prompt: id, enabled: on });
  if (on && !ext.enabled) await api("PATCH", agentPath() + "/extensions", { name: SYSTEM_PROMPTS, enabled: true });
  return prompt;
}

async function togglePrompt(id) {
  const ext = (await getExtensions()).find((e) => e.name === SYSTEM_PROMPTS);
  const current = ext?.prompts?.find((p) => p.id === id);
  if (!current) throw new Error(`no saved system prompt ${id} on this backend`);
  const on = !(ext.enabled && current.enabled);
  const prompt = await setPrompt(id, on);
  const active = (await savedPrompts()).filter((p) => p.active).map((p) => p.title);
  notify(`System prompt "${prompt.title}" is ${on ? "on" : "off"} from this agent's next turn. In effect: ${active.join(", ") || "none"}.`);
}

// ---- settings and info for grok-pi's F2 modal: dpi_settings / dpi_set_setting -----------------------------------------

const sessionPath = () => `/sessions/${encodeURIComponent(session)}`;

// Where the playground is; looked up once a minute at most, since a probe that fails costs its timeout.
let playground = { at: 0, url: null };
async function playgroundUrl() {
  if (Date.now() - playground.at < 60_000) return playground.url;
  const probe = (url) => fetch(url, { signal: AbortSignal.timeout(1500) }).then(() => true, () => false);
  const url = await resolvePlaygroundUrl({ env: process.env, clientEnv, base: BASE, probe });
  playground = { at: Date.now(), url };
  return url;
}

async function snapshot() {
  device = await api("GET", "/device");
  const [agentRow, s, ext, compaction, entries, url] = await Promise.all([
    api("GET", agentPath()),
    api("GET", sessionPath()),
    api("GET", agentPath() + "/extensions"),
    api("GET", sessionPath() + "/compaction").catch(() => undefined),
    activeEntries().catch(() => []),
    playgroundUrl(),
  ]);
  const cfg = agentRow.config;
  const known = device.models.find((m) => m.provider === cfg.model.provider && m.id === cfg.model.modelId);
  const context = contextInfo(usageTokens(modelMessages(entries)), known?.contextWindow);
  const cwd = cfg.cwd ?? s.cwd;
  const info = {
    device: hostName(),
    session: s.name, sessionId: s.id,
    agent: agentRow.name, agentId: String(agentRow.id),
    cwd: tilde(cwd),
    ...(agentRow.branch ? { branch: agentRow.branch } : {}),
    model: `${cfg.model.modelId} (${cfg.model.provider})`,
    thinking: cfg.thinkingLevel ?? "medium",
    readOnly: !!cfg.readOnly,
    tools: (agentRow.tools ?? []).filter((t) => t.offered).map((t) => t.name),
    ...(context ? { context } : {}),
    backendUrl: BASE,
    playgroundUrl: url,
    hints: hintsFor({ device: hostName(), session: s.name, playgroundUrl: url }),
  };
  return { info, options: buildOptions({ agent: agentRow, device, ext, compaction }) };
}

const wantBoolean = (value, what) => {
  if (typeof value !== "boolean") throw new Error(`${what} is on or off, got ${JSON.stringify(value)}`);
  return value;
};

// One setting, by option id, to the backend call the playground makes for it. Nothing here guesses: an id or value
// the backend would not take is an error the person reads.
async function applySetting(id, value) {
  if (id === "model") {
    device = await api("GET", "/device");
    const m = device.models.find((x) => `${x.provider}/${x.id}` === value);
    if (!m) throw new Error(`no model ${JSON.stringify(value)} on this backend`);
    return api("PATCH", agentPath() + "/config", { model: { provider: m.provider, modelId: m.id } });
  }
  if (id === "thinking") {
    if (!THINKING_LEVELS.includes(value)) throw new Error(`thinking is one of ${THINKING_LEVELS.join(", ")}`);
    return api("PATCH", agentPath() + "/config", { thinkingLevel: value });
  }
  if (id === "read_only") return api("PATCH", agentPath() + "/config", { readOnly: wantBoolean(value, "Read-only") });
  if (id.startsWith("prompt:")) return setPrompt(id.slice("prompt:".length), wantBoolean(value, "A system prompt"));
  if (id.startsWith("ext:")) return api("PATCH", agentPath() + "/extensions", { name: id.slice("ext:".length), enabled: wantBoolean(value, "An extension") });
  if (id === "compaction") {
    const a = await api("GET", agentPath());
    const window = device.models.find((m) => m.provider === a.config.model.provider && m.id === a.config.model.modelId)?.contextWindow;
    return api("PATCH", sessionPath() + "/compaction", compactionPatch(window, String(value)));
  }
  throw new Error(`unknown setting ${JSON.stringify(id)}`);
}

// ---- actions: slash commands and shortcut keys ---------------------------------------------------------------------------

const SHORTCUT_DISPATCH = "__pi_shortcut_dispatch";

// The pager matches keys itself and sends the key id back as `/__pi_shortcut_dispatch <key>`.
async function dispatchShortcut(key) {
  const name = commandForKey(key);
  if (!name) throw new Error(`no Durable Pi action on ${key}`);
  await (await shimCommands()).get(name).run("");
}

// The catalog the pager reads (pi_ui.rs `__pi_extension_shortcuts__`); it drops one that arrives before the session
// exists, so it goes out at startup, again when the adapter asks for commands, and every 30s after.
function publishShortcuts() {
  out({ type: "extension_ui_request", id: randomUUID(), method: "setWidget", widgetKey: "__pi_extension_shortcuts__", widgetLines: [JSON.stringify(shortcutCatalog())] });
}

async function showHelp() {
  const commands = [...(await shimCommands())].filter(([name]) => name.startsWith("dpi-")).map(([name, c]) => [name + (c.args ? ` ${c.args}` : ""), c.description]);
  const url = await playgroundUrl();
  const card = helpCard({ device: hostName(), session: (await api("GET", sessionPath())).name, playgroundUrl: url, commands });
  out({ type: "extension_ui_request", id: randomUUID(), method: "dpiHelp", ...card });
}

async function showSettings() {
  const { info, options } = await snapshot();
  const on = options.filter((o) => o.type === "boolean" && o.category !== "Agent" && o.currentValue).length;
  const compact = options.find((o) => o.id === "compaction");
  notify(`Press F2 to change these. Now: ${info.model} · thinking ${info.thinking} · read-only ${info.readOnly ? "on" : "off"} · ${on} prompt${on === 1 ? "" : "s"} and extension${on === 1 ? "" : "s"} on${compact ? ` · auto-compact ${compact.currentValue === "off" ? "off" : `at ${compact.currentValue}%`}` : ""}`);
}

async function showPlayground() {
  const url = await playgroundUrl();
  notify(url ? `Playground: ${url}` : "This backend has no playground.");
}

async function compactNow(instructions) {
  const { taskId } = await api("POST", agentPath() + "/compact", instructions ? { instructions } : {});
  notify(`Compacting this agent's context (task #${taskId}); /dpi-tasks shows how it goes. Stored history stays.`);
}

function resetAgent(note) {
  confirmThen("Reset this agent?", "Starts a fresh context: the agent forgets the conversation so far. Its history stays stored and is still visible in the playground.", async () => {
    await api("POST", agentPath() + "/reset", note ? { note } : {});
    notify("New context started.");
  });
}

// The new agent is a sibling in the same session. The TUI cannot be moved to it from here, so say how to get there.
async function forkAgent(name) {
  const last = (await activeEntries()).filter((e) => e.kind === "pi.assistant").at(-1);
  if (!last) throw new Error("nothing to fork yet: the agent has not replied");
  const forked = await api("POST", agentPath() + "/fork", { entryId: last.id, ...(name ? { name } : {}) });
  notify(`Forked at reply #${last.id} as agent ${forked.name ?? forked.id}. This one is untouched; ctrl+r switches to the fork.`);
}

async function newAgent(name) {
  const created = await api("POST", sessionPath() + "/agents", { from: agent, ...(name ? { name } : {}) });
  notify(`New agent ${created.name ?? created.id} in this session, with this agent's model and settings. ctrl+r switches to it.`);
}

// The main agent is the session: its name is the session's. Any other agent has a name of its own.
async function renameAgent(name) {
  if (!name) throw new Error("usage: /dpi-rename <name>");
  const a = await api("GET", agentPath());
  if (a.kind === "root") await api("PATCH", sessionPath(), { name });
  else await api("PATCH", agentPath(), { name });
  notify(`${a.kind === "root" ? "Session" : "Agent"} renamed to ${name}.`);
}

async function stopAgent() {
  await api("POST", agentPath() + "/abort", { background: true });
  notify("Stopped: the running turn and its background tasks.");
}

function archiveSession() {
  confirmThen("Archive this session?", "Stops its agents and moves it out of the list. Nothing is deleted, and the playground can restore it.", async () => {
    await api("DELETE", sessionPath());
    stream?.abort(); // the session is gone: stop polling it
    notify("Session archived. This window no longer has a session; ctrl+r picks another, or quit.");
  });
}

async function listSessions() {
  const sessions = await api("GET", "/sessions");
  const rows = sessions.map((s) => {
    const agents = s.agents ?? [];
    const busy = agents.filter((a) => a.busy).length;
    return `${s.id === session ? "▸" : " "} ${s.name} · ${agents.length} agent${agents.length === 1 ? "" : "s"}${busy ? ` · ${busy} running` : ""} · ${tilde(s.cwd)}`;
  });
  notify([`${sessions.length} session${sessions.length === 1 ? "" : "s"} on ${hostName()} (▸ this one; ctrl+r switches):`, ...rows].join("\n"));
}

const taskLine = (t) => {
  const o = t.state?.outcome;
  return `#${t.id} ${t.kind} ${o ? o.status : t.state?.status ?? ""}${o?.error?.message ? `: ${o.error.message}` : ""}`;
};
async function listTasks() {
  const base = `/sessions/${encodeURIComponent(session)}`;
  const [live, done] = await Promise.all([api("GET", base + "/tasks?status=live"), api("GET", base + "/tasks?status=terminal&limit=50")]);
  // Every turn is a pi.generation task; the rest (tools, jobs, subagents) is what someone looks for.
  const recent = done.filter((t) => t.kind !== "pi.generation").slice(0, 8);
  notify([
    `Live tasks: ${live.length ? "" : "none"}`, ...live.map((t) => `  ${taskLine(t)}`),
    `Recent: ${recent.length ? "" : "none"}`, ...recent.map((t) => `  ${taskLine(t)}`),
  ].join("\n"));
}

async function listDeploys() {
  let deploys;
  try { deploys = await ops("GET", "/deploys"); } catch (err) {
    return notify(`No ops service answering at ${OPS_URL} (${err.message}), so no deploy approvals to show.`);
  }
  const waiting = deploys.filter((d) => d.status === "awaiting_approval");
  const open = new Set([...dialogs.values()].map((d) => d.key));
  // A dismissed dialog waits out its cooldown; asking for the list reopens it on the next poll.
  for (const d of waiting) if (!open.has(deployKey(d))) asked.delete(deployKey(d));
  notify(waiting.length
    ? [`${waiting.length} deploy${waiting.length > 1 ? "s" : ""} awaiting approval (dialogs open on the next poll):`, ...waiting.map((d) => `  ${d.id} ${d.incidentId}: ${d.summary} (${d.branch} ${d.commit})`)].join("\n")
    : "No deploys awaiting approval.");
}

// true when `message` was one of the shim's commands (and it ran), false for anything to send to the agent.
async function runCommand(message) {
  const m = typeof message === "string" && message.match(/^\/(\S+)\s*([\s\S]*)$/);
  if (!m) return false;
  const cmd = (await shimCommands()).get(m[1]);
  if (!cmd) return false;
  if (m[1] === CONTEXT_BREAKDOWN) await cmd.run(); // a failure is the adapter's to handle: /context falls back quietly
  else await cmd.run(m[2].trim()).catch((err) => notify(`/${m[1]}: ${err.message}`, "error"));
  return true;
}

const handlers = {
  get_state: getState,
  // Re-read so model list edits on the backend show without restarting grok-pi.
  get_available_models: async () => ((device = await api("GET", "/device")), { models: device.models }),
  get_commands: async () => {
    const commands = [...(await shimCommands())].map(([name, c]) => ({ name, description: c.description, source: "extension" }));
    setTimeout(publishShortcuts, 0); // after this answer: the adapter has the session by now
    return { commands };
  },
  dpi_settings: snapshot,
  dpi_set_setting: async (c) => {
    await applySetting(String(c.settingId ?? ""), c.value);
    return snapshot();
  },
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
  // The dashboard's stop on a row this TUI is not attached to: abort that agent's turn, stay where we are.
  dpi_abort: async (c) => {
    // target: a roster row id "<session>:<agent>" (dpi_list_sessions) or a dpi:// path
    const target = String(c.target ?? "");
    const row = target.match(/^([^/:]+):([^/:]+)$/);
    const [s, a] = row ? [row[1], row[2]] : parseTarget(target);
    await api("POST", `/sessions/${encodeURIComponent(s)}/agents/${encodeURIComponent(a)}/abort`, {});
  },
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
  // grok-pi's /resume picker and the Agent Dashboard roster (GROK_PI_DPI=1): one row per durable agent, with its live
  // activity from the backend's busy flag. The tree has no activity time or counts, so each row reads its agent's
  // active transcript; the dashboard polls every second, so an idle agent's transcript is reused (see listedEntries).
  dpi_remote: async (c) => { const r = await remote(!!c.card); sentRemote(r); return r; },
  dpi_list_sessions: async () => {
    const rows = (await api("GET", "/sessions")).flatMap((s) => (s.agents ?? []).map((a) => [s, a]));
    const active = await Promise.all(rows.map(([s, a]) => listedEntries(s, a)));
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
          activity: a.busy ? "working" : "idle",
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
  publishShortcuts();
  setInterval(publishShortcuts, 30_000).unref();
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
