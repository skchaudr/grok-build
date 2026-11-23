// node --test tools/dpi-rpc/ : drives dpi-rpc.mjs over stdin/stdout against a fake dpi HTTP server.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolvePlaygroundUrl, buildOptions, compactionPatch, SHORTCUTS } from "./settings.mjs";

const SHIM = join(dirname(fileURLToPath(import.meta.url)), "dpi-rpc.mjs");
const TOKEN = "test-token";
const msg = (role, ts, content = "x") => ({ role, content, timestamp: ts });
const tool = (name) => ({ name, description: `${name} tool`, parameters: { type: "object" } });
const agents1 = [
  { id: 1, name: "Main", kind: "root", busy: false },
  { id: 2, name: "Alt", kind: "fork", parentId: 1, busy: true },
  { id: 3, name: "Sub", kind: "subagent", ownerAgentId: 2, busy: false },
];
const state = { tasks: [], doneTasks: [], prime: null, requests: [], ext: null, branch: "main", streams: [], model: { provider: "p", modelId: "m" }, thinking: "low", readOnly: false, compaction: { enabled: true, reserveTokens: 20000, backgroundTokens: 30000, keepRecentTokens: 2000 } };
const resetAgentState = () => Object.assign(state, { model: { provider: "p", modelId: "m" }, thinking: "low", readOnly: false, compaction: { enabled: true, reserveTokens: 20000, backgroundTokens: 30000, keepRecentTokens: 2000 }, failConfig: false });
const freshExt = () => ({
  builtIn: ["coding-tools"],
  extensions: [{
    name: "system-prompts", title: "System prompts", enabled: false,
    prompts: [
      { id: "smart_ass_opus_fix.md", title: "Smart Ass Opus fix", enabled: true },
      { id: "one_sentence.md", title: "One sentence", enabled: false },
    ],
  }],
});
const entries = {
  "s1/1": [
    { id: 4, kind: "pi.system", model: [{ ...msg("system", 1500), sections: { a: "aaaa", b: "bbbbbbbb" }, toolsAdded: [tool("read"), tool("bash")] }] },
    { id: 5, kind: "pi.user", model: [msg("user", 2000, "first question")] },
    { id: 6, kind: "pi.assistant", model: [msg("assistant", 3000)] },
    { id: 7, kind: "pi.user", model: [msg("user", 4000, [{ type: "text", text: "second" }])] },
    // a later patch: drops section a, adds c, swaps the read tool for edit
    { id: 11, kind: "pi.system", model: [{ ...msg("system", 4500, ""), sections: { a: null, c: "cc" }, toolsRemoved: [{ name: "read" }], toolsAdded: [tool("edit")] }] },
  ],
  "s1/3": [{ id: 8, kind: "pi.user", model: [msg("user", 5000)] }],
  // an agent whose last reply reported usage: 100 + 50 + 50 tokens
  "s1/4": [{ id: 20, kind: "pi.assistant", model: [{ ...msg("assistant", 6000), usage: { input: 100, output: 50, cacheRead: 50 } }] }],
};

const json = (res, code, body) => res.writeHead(code, { "content-type": "application/json" }).end(JSON.stringify(body));
const server = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (req.headers.authorization !== `Bearer ${TOKEN}`) return json(res, 401, { error: { message: "bad token" } });
    const url = new URL(req.url, "http://x");
    const p = url.pathname.replace(/^\/v1/, "");
    state.requests.push(`${req.method} ${p}${url.search}${body ? " " + body : ""}`);
    let m;
    if (p === "/device") return json(res, 200, { hostname: "mini-host", models: [{ provider: "p", id: "m", contextWindow: 1000, reasoning: false }, { provider: "p", id: "big", contextWindow: 200000, reasoning: true }, { provider: "q", id: "a/b:free", contextWindow: 200000 }] });
    if (p === "/sessions/s1/compaction") {
      if (req.method === "PATCH") { Object.assign(state.compaction, JSON.parse(body)); if (state.failCompaction) return json(res, 400, { error: { message: "reserveTokens must be an integer from 1024 to 1000000" } }); }
      return json(res, 200, state.compaction);
    }
    if (p === "/sessions" && req.method === "GET") {
      return json(res, 200, [
        { id: "s1", name: "one", cwd: "/w", model: "m", createdAt: 1000, agents: agents1 },
        { id: "s2", name: "two", cwd: "/home/bob/two", model: "m", createdAt: 9000, agents: [{ id: 1, name: "Main", kind: "root", busy: false }] },
      ]);
    }
    if (p === "/sessions" && req.method === "POST") return json(res, 201, { id: "s3", agents: [{ id: 1 }] });
    if (p === "/sessions/s1/agents" && req.method === "POST") return json(res, 201, { id: 10 });
    if (req.method === "DELETE" && state.failDelete) return json(res, 409, { error: { message: "session is busy" } });
    if ((m = p.match(/^\/sessions\/(s\d)$/))) return json(res, 200, { id: m[1], name: m[1], cwd: "/w" });
    if (p.endsWith("/tasks") && state.failTasks) return json(res, 500, { error: { message: "tasks down" } });
    if (p.endsWith("/tasks")) return json(res, 200, url.search.includes("terminal") ? state.doneTasks : state.tasks);
    if (p.endsWith("/prime/work")) return state.prime ? json(res, 200, state.prime) : json(res, 404, { error: { message: "no such route" } });
    if ((m = p.match(/^\/sessions\/(s\d)\/agents\/(\w+)(\/.*)?$/))) {
      const [, s, a, rest = ""] = m;
      const id = a === "main" ? 1 : Number(a);
      if (rest === "/events") {
        res.writeHead(200, { "content-type": "text/event-stream" });
        state.streams.push(res);
        return res.write(`data: ${JSON.stringify({ type: "snapshot", snapshot: { entries: [] } })}\n\n`);
      }
      if (rest === "/entries") return entries[`${s}/${id}`] ? json(res, 200, { items: entries[`${s}/${id}`] }) : json(res, 500, { error: { message: "boom" } });
      if (rest === "/fork") return json(res, 201, { id: 9, name: JSON.parse(body || "{}").name ?? "9" });
      if (rest === "/compact") return json(res, 200, { taskId: 42 });
      if (rest === "/reset" || rest === "/abort") return json(res, 200, { ok: true });
      if (rest === "/config" && req.method === "PATCH") {
        const b = JSON.parse(body);
        if (state.failConfig) return json(res, 400, { error: { message: "Invalid thinking level" } });
        if (b.model) state.model = b.model;
        if (b.thinkingLevel) state.thinking = b.thinkingLevel;
        if (b.readOnly !== undefined) state.readOnly = b.readOnly;
        return json(res, 200, {});
      }
      if (rest === "/extensions" && req.method === "PATCH") {
        const b = JSON.parse(body);
        const ext = state.ext.extensions[0];
        if (b.name) ext.enabled = b.enabled;
        else ext.prompts.find((x) => x.id === b.prompt).enabled = b.enabled;
        return json(res, 200, state.ext);
      }
      if (rest === "/extensions") return json(res, 200, state.ext);
      if (rest === "") return json(res, 200, { id, sessionId: s, name: "n", kind: id === 2 ? "fork" : "root", busy: false, config: { model: state.model, cwd: id === 3 ? "/Users/bob/repo" : "/w/proj", thinkingLevel: state.thinking, readOnly: state.readOnly }, tools: [{ name: "read", offered: true }, { name: "edit", offered: !state.readOnly }], ...(state.branch && id === 1 ? { branch: state.branch } : {}) });
    }
    json(res, 404, { error: { message: `fake: ${req.method} ${p}` } });
  });
});
let base;
before(async () => { await new Promise((r) => server.listen(0, "127.0.0.1", r)); base = `http://127.0.0.1:${server.address().port}`; });
after(() => { server.closeAllConnections(); server.close(); });

// Spawn the shim attached to s1; returns call(cmd) and the extension_ui_request frames seen so far.
async function shim(env = {}, target = "s1") {
  const child = spawn("node", [SHIM, "--session", target], {
    env: { PATH: process.env.PATH, HOME: mkdtempSync(join(tmpdir(), "dpi-rpc-")), DPI_URL: base, DPI_TOKEN: TOKEN, DPI_TASK_POLL_MS: "50", DPI_OPS_POLL_MS: "50", DPI_OPS_URL: "http://127.0.0.1:1", ...env },
  });
  const lines = [];
  const waiters = [];
  createInterface({ input: child.stdout }).on("line", (l) => {
    lines.push(JSON.parse(l));
    waiters.splice(0).forEach((w) => w());
  });
  const until = async (pred) => {
    const deadline = Date.now() + 3000;
    for (;;) {
      const hit = lines.find(pred);
      if (hit) return hit;
      if (Date.now() > deadline) throw new Error(`timeout; saw ${JSON.stringify(lines)}`);
      await new Promise((r) => { waiters.push(r); setTimeout(r, 100); });
    }
  };
  let n = 0;
  const call = async (cmd) => {
    const id = `c${++n}`;
    child.stdin.write(JSON.stringify({ ...cmd, id }) + "\n");
    return until((l) => l.type === "response" && l.id === id);
  };
  const status = (key, text) => until((l) => l.method === "setStatus" && l.statusKey === key && l.statusText === text);
  const send = (obj) => child.stdin.write(JSON.stringify(obj) + "\n");
  return { call, status, until, lines, send, close: () => child.kill() };
}

test("dpi_list_sessions fills parent, message count and activity time", async () => {
  const s = await shim();
  try {
    const { data } = await s.call({ type: "dpi_list_sessions" });
    const row = (id) => data.sessions.find((r) => r.id === id);
    assert.equal(row("s1:1").messageCount, 3); // system entry excluded
    assert.equal(row("s1:1").modifiedAt, new Date(4000).toISOString());
    assert.equal(row("s1:1").parentSessionPath, undefined);
    assert.equal(row("s1:2").parentSessionPath, "dpi://s1/1"); // fork -> its parent agent
    assert.equal(row("s1:3").parentSessionPath, "dpi://s1/2"); // subagent -> its owner
    assert.equal(row("s1:3").messageCount, 1);
    assert.equal(row("s1:2").messageCount, 0); // entries fetch failed: row still listed
    assert.equal(row("s1:2").modifiedAt, new Date(1000).toISOString());
    assert.equal(row("s2:1").modifiedAt, new Date(9000).toISOString());
  } finally { s.close(); }
});

test("/context: get_commands registers the breakdown bridge, and running it writes prompt and tool estimates without messaging the agent", async () => {
  const out = join(mkdtempSync(join(tmpdir(), "dpi-rpc-")), "breakdown.json");
  const s = await shim({ PI_GROK_CONTEXT_BREAKDOWN: out });
  try {
    const { data } = await s.call({ type: "get_commands" });
    assert.ok(data.commands.some((c) => c.name === "__pi_context_breakdown" && c.source === "extension"));
    state.requests.length = 0;
    assert.equal((await s.call({ type: "prompt", message: "/__pi_context_breakdown" })).success, true);
    const defs = [tool("bash"), tool("edit")];
    assert.deepEqual(JSON.parse(readFileSync(out, "utf8")), {
      version: 1,
      systemPromptTokensRaw: Math.ceil("x\n\nbbbbbbbb\n\ncc".length / 4), // content, then sections in effect
      toolDefinitionsCount: 2,
      toolDefinitionsTokensRaw: Math.ceil(JSON.stringify(defs).length / 4),
      appendTokensRaw: 0, contextFiles: [], skillsCount: 0, skillsTokensRaw: 0,
    });
    assert.equal(state.requests.some((q) => q.includes("/messages")), false);
  } finally { s.close(); }
});

test("a prompt that is not a shim command goes to the agent, slash and all", async () => {
  const s = await shim();
  try {
    state.requests.length = 0;
    await s.call({ type: "prompt", message: "/not-a-command hi" });
    assert.ok(state.requests.some((q) => q.startsWith("POST /sessions/s1/agents/1/messages") && q.includes('"content":"/not-a-command hi"')));
  } finally { s.close(); }
});

const notified = (s, text) => s.until((l) => l.method === "notify" && l.message === text);

test("get_commands lists the dpi commands and one toggle per saved system prompt, all run by the shim", async () => {
  state.ext = freshExt();
  const s = await shim();
  try {
    const { data } = await s.call({ type: "get_commands" });
    const names = data.commands.map((c) => c.name);
    assert.deepEqual(names, [
      "__pi_context_breakdown", "__pi_shortcut_dispatch", "dpi-help", "dpi-settings", "dpi-open", "dpi-compact", "dpi-reset", "dpi-fork",
      "dpi-new-agent", "dpi-rename", "dpi-stop", "dpi-archive", "dpi-sessions", "dpi-tasks", "dpi-deploys", "prompt-smart-ass-opus-fix", "prompt-one-sentence",
    ]);
    assert.ok(data.commands.every((c) => c.source === "extension" && c.description));
  } finally { s.close(); }
});

test("a saved-prompt command turns the prompt on (with its extension) and off again, and says what is in effect", async () => {
  state.ext = freshExt();
  const s = await shim();
  try {
    state.requests.length = 0;
    await s.call({ type: "prompt", message: "/prompt-one-sentence" });
    await notified(s, 'System prompt "One sentence" is on from this agent\'s next turn. In effect: Smart Ass Opus fix, One sentence.');
    assert.deepEqual(state.requests.filter((q) => q.startsWith("PATCH")), [
      'PATCH /sessions/s1/agents/1/extensions {"prompt":"one_sentence.md","enabled":true}',
      'PATCH /sessions/s1/agents/1/extensions {"name":"system-prompts","enabled":true}',
    ]);
    await s.call({ type: "prompt", message: "/prompt-one-sentence" });
    await notified(s, 'System prompt "One sentence" is off from this agent\'s next turn. In effect: Smart Ass Opus fix.');
    assert.equal(state.ext.extensions[0].enabled, true); // the other checked prompt stays on
    assert.equal(state.requests.some((q) => q.includes("/messages")), false);
  } finally { s.close(); }
});

test("/dpi-sessions lists every session and marks the attached one", async () => {
  const s = await shim();
  try {
    await s.call({ type: "prompt", message: "/dpi-sessions" });
    await notified(s, "2 sessions on mini-host (▸ this one; ctrl+r switches):\n▸ one · 3 agents · 1 running · /w\n  two · 1 agent · ~/two");
  } finally { s.close(); }
});

test("/dpi-tasks shows live tasks and recent non-turn tasks with their outcome", async () => {
  state.doneTasks = [
    { id: 9, kind: "pi.generation", state: { status: "terminal", outcome: { status: "completed" } } },
    { id: 8, kind: "pi.tool", state: { status: "terminal", outcome: { status: "failed", error: { message: "Tool bash threw" } } } },
  ];
  const s = await shim();
  try {
    await s.call({ type: "prompt", message: "/dpi-tasks" });
    await notified(s, "Live tasks: none\nRecent: \n  #8 pi.tool failed: Tool bash threw");
  } finally { s.close(); state.doneTasks = []; }
});

test("/dpi-deploys says when no ops service answers", async () => {
  const s = await shim({ DPI_OPS_URL: "http://127.0.0.1:1" });
  try {
    await s.call({ type: "prompt", message: "/dpi-deploys" });
    await s.until((l) => l.method === "notify" && l.message.startsWith("No ops service answering at http://127.0.0.1:1 ("));
  } finally { s.close(); }
});

test("a shim command that fails shows its error instead of going quiet", async () => {
  state.failTasks = true;
  const s = await shim();
  try {
    assert.equal((await s.call({ type: "prompt", message: "/dpi-tasks" })).success, true);
    const n = await s.until((l) => l.method === "notify" && l.notifyType === "error");
    assert.equal(n.message, "/dpi-tasks: tasks down");
  } finally { s.close(); state.failTasks = false; }
});

test("dpi_remote names the backend host, the agent's real cwd and branch; home folds to ~", async () => {
  const s = await shim();
  try {
    assert.deepEqual((await s.call({ type: "dpi_remote" })).data, { host: "mini-host", cwd: "/w/proj", label: "mini-host:/w/proj", branch: "main" });
  } finally { s.close(); }
  const sub = await shim({}, "dpi://s1/3");
  try {
    assert.deepEqual((await sub.call({ type: "dpi_remote" })).data, { host: "mini-host", cwd: "/Users/bob/repo", label: "mini-host:~/repo" }); // no branch: none shown
  } finally { sub.close(); }
});

test("dpi_remote with card: the attach card's title and rows come from the backend", async () => {
  state.tasks = [{ id: 2, kind: "job", state: { status: "running" } }];
  const s = await shim();
  try {
    const { card } = (await s.call({ type: "dpi_remote", card: true })).data;
    assert.equal(card.title, "dpi · s1 on mini-host");
    const rows = Object.fromEntries(card.rows);
    assert.deepEqual(Object.keys(rows), ["model", "cwd", "activity", "last ask", "tasks"]);
    assert.equal(rows.model, "m (p) · thinking low");
    assert.equal(rows.cwd, "mini-host:/w/proj · main");
    assert.match(rows.activity, /^last message \d+d ago · 3 messages$/);
    assert.equal(rows["last ask"], "second");
    assert.equal(rows.tasks, "1 task: job");
    assert.equal(card.playgroundUrl, null); // nothing answers on :4781 of the fake backend's host
  } finally { s.close(); state.tasks = []; }
  const hub = await shim({ DPI_HUB_URL: "http://hub.example:4781/" });
  try {
    assert.equal((await hub.call({ type: "dpi_remote", card: true })).data.card.playgroundUrl, "http://hub.example:4781");
  } finally { hub.close(); }
});

test("after a turn ends, a changed branch is pushed as setRemote; an unchanged one is not", async () => {
  state.streams.length = 0;
  const s = await shim();
  try {
    await s.call({ type: "dpi_remote" });
    await s.until(() => state.streams.length > 0);
    const runEnd = () => state.streams.at(-1).write(`data: ${JSON.stringify({ type: "batch", events: [{ type: "run_end" }] })}\n\n`);
    runEnd();
    await s.until((l) => l.type === "agent_settled");
    state.branch = "sab/other";
    runEnd();
    const push = await s.until((l) => l.method === "setRemote");
    assert.deepEqual([push.label, push.branch], ["mini-host:/w/proj", "sab/other"]);
    await settle();
    assert.equal(s.lines.filter((l) => l.method === "setRemote").length, 1);
  } finally { s.close(); state.branch = "main"; }
});

test("get_fork_messages lists user turns by dpi entry id", async () => {
  const s = await shim();
  try {
    const { data } = await s.call({ type: "get_fork_messages" });
    assert.deepEqual(data.messages, [{ entryId: "5", text: "first question" }, { entryId: "7", text: "second" }]);
  } finally { s.close(); }
});

test("fork branches before the picked user turn, returns its text, and follows the fork", async () => {
  const s = await shim();
  try {
    assert.equal((await s.call({ type: "fork", entryId: "6" })).success, false); // not a user turn
    const r = await s.call({ type: "fork", entryId: "7" });
    assert.deepEqual(r.data, { cancelled: false, text: "second" });
    assert.ok(state.requests.includes('POST /sessions/s1/agents/1/fork {"entryId":6}'));
    assert.equal((await s.call({ type: "get_state" })).data.sessionId, "s1:9");
  } finally { s.close(); }
});

test("fork of an agent's first turn starts a fresh agent from it", async () => {
  const s = await shim({}, "dpi://s1/3");
  try {
    const r = await s.call({ type: "fork", entryId: "8" });
    assert.equal(r.data.cancelled, false);
    assert.ok(state.requests.includes('POST /sessions/s1/agents {"from":"3"}'));
    assert.equal((await s.call({ type: "get_state" })).data.sessionId, "s1:10");
  } finally { s.close(); }
});

test("new_session creates a session with the current cwd and model, then attaches", async () => {
  const s = await shim();
  try {
    const r = await s.call({ type: "new_session" });
    assert.deepEqual(r.data, { cancelled: false });
    const post = state.requests.findLast((q) => q.startsWith("POST /sessions {"));
    const body = JSON.parse(post.slice("POST /sessions ".length));
    assert.match(body.name, /^proj \d\d-\d\d \d\d:\d\d$/);
    delete body.name;
    assert.deepEqual(body, { cwd: "/w/proj", model: { provider: "p", modelId: "m" }, thinkingLevel: "low" });
    assert.equal((await s.call({ type: "get_state" })).data.sessionId, "s3:1");
  } finally { s.close(); }
});

test("live tasks and Prime work surface as setStatus lines, and clear", async () => {
  state.tasks = [
    { id: 1, kind: "pi.tool", background: false, label: "bash" }, // foreground tool: not news
    { id: 2, kind: "pi.tool", background: true, label: "sleep 60" },
    { id: 3, kind: "job", background: false },
  ];
  state.prime = { goal: { status: "active", objective: "ship it" }, heartbeat: { status: "active", schedule: "every 1h" }, schedules: [] };
  const s = await shim();
  try {
    await s.status("dpi-tasks", "2 tasks: sleep 60, job");
    await s.status("prime-work", "goal active: ship it | heartbeat active every 1h");
    state.tasks = [];
    await s.status("dpi-tasks", undefined);
    const frames = s.lines.filter((l) => l.statusKey === "dpi-tasks");
    assert.equal(frames.length, 2); // unchanged polls emit nothing
  } finally { s.close(); state.prime = null; }
});

test("a backend without prime/work (404) gets no prime status and is not polled again", async () => {
  state.tasks = [{ id: 2, kind: "x", background: true }];
  state.requests.length = 0;
  const s = await shim();
  try {
    await s.status("dpi-tasks", "1 task: x");
    await new Promise((r) => setTimeout(r, 250));
    assert.equal(s.lines.filter((l) => l.statusKey === "prime-work").length, 0);
    assert.equal(state.requests.filter((q) => q.includes("/prime/work")).length, 1);
  } finally { s.close(); state.tasks = []; }
});

// dpi-grok exports DPI_URL/DPI_TOKEN for DPI_BACKEND=prime; the shim only has to honour env over ~/.dpi.
test("DPI_URL/DPI_TOKEN from env win: the right token attaches, a wrong one exits", async () => {
  const s = await shim();
  try { assert.equal((await s.call({ type: "get_state" })).success, true); } finally { s.close(); }
  const bad = spawn("node", [SHIM, "--session", "s1"], { env: { PATH: process.env.PATH, HOME: mkdtempSync(join(tmpdir(), "dpi-rpc-")), DPI_URL: base, DPI_TOKEN: "wrong" } });
  const code = await new Promise((r) => bad.on("exit", r));
  assert.equal(code, 1);
});

// ---- deploy approvals over a fake ops service ----
const deploy = (over = {}) => ({ id: 1, requestedAt: 1000, incidentId: "INC-0001", status: "awaiting_approval", commit: "abc123", branch: "fix/x", summary: "retry fix", diffStat: "1 file changed", ...over });
const ops = { deploys: [], decisions: [], failDecide: false };
const opsServer = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (req.method === "GET" && req.url === "/api/deploys") return json(res, 200, ops.deploys);
    const m = req.url.match(/^\/api\/deploys\/(\d+)\/decide$/);
    if (req.method === "POST" && m && ops.failDecide) return json(res, 409, { error: "already decided" });
    if (req.method === "POST" && m) { ops.decisions.push([Number(m[1]), JSON.parse(body)]); return json(res, 200, { ok: true }); }
    json(res, 404, {});
  });
});
let opsUrl;
before(async () => { await new Promise((r) => opsServer.listen(0, "127.0.0.1", r)); opsUrl = `http://127.0.0.1:${opsServer.address().port}`; });
after(() => { opsServer.closeAllConnections(); opsServer.close(); });
const settle = () => new Promise((r) => setTimeout(r, 300));
const isConfirm = (l) => l.method === "confirm";

test("an awaiting deploy gets one confirm dialog and an ops-deploy status line; approve posts approved:true", async () => {
  Object.assign(ops, { deploys: [deploy()], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl });
  try {
    const q = await s.until(isConfirm);
    assert.equal(q.title, "Deploy INC-0001: retry fix");
    assert.equal(q.message, "Deploy INC-0001: retry fix\ncommit abc123\nbranch fix/x\n1 file changed\nIf this was already decided elsewhere (hub, another device) you can dismiss this; dismissing records nothing.");
    assert.equal(q.timeout, 300000); // the adapter retracts the dialog itself after this
    await s.status("ops-deploy", "1 deploy awaiting approval");
    await settle();
    assert.equal(s.lines.filter(isConfirm).length, 1); // polling again does not re-prompt
    s.send({ type: "extension_ui_response", id: q.id, confirmed: true });
    await s.until(() => ops.decisions.length > 0);
    assert.deepEqual(ops.decisions, [[1, { approved: true }]]);
    await s.until((l) => l.method === "notify" && l.message === "Deploy 1 approved."); // only after decide returned ok
    ops.deploys = [deploy({ status: "deploying" })];
    await s.status("ops-deploy", undefined);
    await settle();
    assert.equal(s.lines.filter(isConfirm).length, 1); // no re-prompt after the decision
    assert.equal(ops.decisions.length, 1);
  } finally { s.close(); }
});

test("an explicit No posts approved:false", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 2 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl });
  try {
    s.send({ type: "extension_ui_response", id: (await s.until(isConfirm)).id, confirmed: false });
    await s.until(() => ops.decisions.length > 0);
    assert.deepEqual(ops.decisions, [[2, { approved: false }]]);
    await s.until((l) => l.method === "notify" && l.message === "Deploy 2 declined.");
  } finally { s.close(); }
});

test("a decide that fails is reported as not recorded, never as approved", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 6 })], decisions: [], failDecide: true });
  const s = await shim({ DPI_OPS_URL: opsUrl });
  try {
    s.send({ type: "extension_ui_response", id: (await s.until(isConfirm)).id, confirmed: true });
    await s.until((l) => l.method === "notify" && l.message === "Deploy 6: could not record your answer: POST /deploys/6/decide: HTTP 409");
    assert.equal(s.lines.some((l) => l.method === "notify" && /approved/.test(l.message)), false);
  } finally { s.close(); ops.failDecide = false; }
});

test("a deploy id reused after a demo reset is prompted again", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 15, requestedAt: 1 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl });
  try {
    const first = await s.until(isConfirm);
    s.send({ type: "extension_ui_response", id: first.id, confirmed: true });
    await s.until(() => ops.decisions.length > 0);
    ops.deploys = [deploy({ id: 15, requestedAt: 2 })]; // reset: same id, a new request
    const second = await s.until((l) => isConfirm(l) && l.id !== first.id);
    assert.equal(second.title, "Deploy INC-0001: retry fix");
  } finally { s.close(); }
});

test("a dismissed dialog records nothing, notifies, stays pending, and re-prompts only after the cooldown", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 4 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl, DPI_OPS_REPROMPT_MS: "700" });
  try {
    const first = await s.until(isConfirm);
    const t0 = Date.now();
    s.send({ type: "extension_ui_response", id: first.id, cancelled: true });
    await s.until((l) => l.method === "notify" && l.message === "No decision recorded for deploy 4; it is still awaiting approval");
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(s.lines.filter(isConfirm).length, 1); // inside the cooldown: no prompt loop
    const second = await s.until((l) => isConfirm(l) && l.id !== first.id);
    assert.ok(Date.now() - t0 >= 650, "re-prompted before the cooldown");
    s.send({ type: "extension_ui_response", id: second.id }); // no confirmed at all: still no decision
    await new Promise((r) => setTimeout(r, 200));
    assert.deepEqual(ops.decisions, []);
    assert.equal(s.lines.filter((l) => l.statusKey === "ops-deploy").at(-1).statusText, "1 deploy awaiting approval");
  } finally { s.close(); }
});

test("/dpi-deploys lists pending deploys and reopens a dismissed dialog without waiting out the cooldown", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 7 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl, DPI_OPS_REPROMPT_MS: "600000" });
  try {
    const first = await s.until(isConfirm);
    s.send({ type: "extension_ui_response", id: first.id, cancelled: true });
    await s.until((l) => l.method === "notify" && l.message.startsWith("No decision recorded for deploy 7"));
    await s.call({ type: "prompt", message: "/dpi-deploys" });
    await s.until((l) => l.method === "notify" && l.message === "1 deploy awaiting approval (dialogs open on the next poll):\n  7 INC-0001: retry fix (fix/x abc123)");
    await s.until((l) => isConfirm(l) && l.id !== first.id);
  } finally { s.close(); }
});

test("no dialog and no noise when ops is down", async () => {
  const s = await shim({ DPI_OPS_URL: "http://127.0.0.1:1" });
  try {
    await s.call({ type: "get_state" });
    await settle();
    assert.equal(s.lines.filter((l) => isConfirm(l) || l.statusKey === "ops-deploy").length, 0);
  } finally { s.close(); }
});

test("a deploy decided elsewhere is captioned with how and when; a late Yes or No is ignored and says so", async () => {
  const decidedAt = Date.parse("2026-11-15T20:24:00");
  const at = "20:24";
  for (const [answer, word] of [[true, "Yes"], [false, "No"]]) {
    Object.assign(ops, { deploys: [deploy({ id: 15 })], decisions: [] });
    const s = await shim({ DPI_OPS_URL: opsUrl });
    try {
      const q = await s.until(isConfirm);
      ops.deploys = [deploy({ id: 15, status: "deploying", decidedAt })];
      await s.until((l) => l.method === "notify" && l.message === `Deploy 15 was approved elsewhere at ${at}. You can dismiss the open dialog; nothing was recorded from it.`);
      s.send({ type: "extension_ui_response", id: q.id, confirmed: answer });
      await s.until((l) => l.method === "notify" && l.message === `Deploy 15 was already approved at ${at}; your ${word} was not sent.`);
      await settle();
      assert.deepEqual(ops.decisions, []);
    } finally { s.close(); }
  }
});

test("a deploy declined elsewhere is captioned as declined; a stale dialog that times out stays silent", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 16 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl });
  try {
    const q = await s.until(isConfirm);
    ops.deploys = [deploy({ id: 16, status: "declined", decidedAt: null })];
    await s.until((l) => l.method === "notify" && l.message.startsWith("Deploy 16 was declined elsewhere. You can dismiss"));
    const notifies = s.lines.filter((l) => l.method === "notify").length;
    s.send({ type: "extension_ui_response", id: q.id, cancelled: true });
    await settle();
    assert.equal(s.lines.filter((l) => l.method === "notify").length, notifies);
    assert.deepEqual(ops.decisions, []);
  } finally { s.close(); }
});

test("a dialog the adapter retracted on its timeout is shown again at once, not after the cooldown", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 5 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl, DPI_OPS_DIALOG_MS: "300", DPI_OPS_REPROMPT_MS: "600000" });
  try {
    const first = await s.until(isConfirm);
    assert.equal(first.timeout, 300);
    await new Promise((r) => setTimeout(r, 350));
    s.send({ type: "extension_ui_response", id: first.id, cancelled: true }); // what the adapter sends after retracting
    await s.until((l) => isConfirm(l) && l.id !== first.id);
    assert.equal(s.lines.some((l) => l.method === "notify"), false); // a timeout is not a dismissal
    assert.deepEqual(ops.decisions, []);
  } finally { s.close(); }
});

// ---- dpi-grok's pre-check ----
function runShim(args) {
  const child = spawn("node", [SHIM, ...args], { env: { PATH: process.env.PATH, HOME: mkdtempSync(join(tmpdir(), "dpi-rpc-")), DPI_URL: base, DPI_TOKEN: TOKEN } });
  let stdout = "", stderr = "";
  child.stdout.on("data", (c) => (stdout += c));
  child.stderr.on("data", (c) => (stderr += c));
  return new Promise((r) => child.on("exit", (code) => r({ code, stdout, stderr })));
}

test("--resolve prints session:agent for a known name, and names the cause for an unknown one", async () => {
  const ok = await runShim(["--resolve", "--session", "s1"]);
  assert.deepEqual([ok.code, ok.stdout], [0, "s1:1\n"]);
  const bad = await runShim(["--resolve", "--session", "zzz"]);
  assert.equal(bad.code, 1);
  assert.match(bad.stderr, /no dpi session named zzz on http:\/\/127\.0\.0\.1:\d+/);
});

// ---- settings, info, actions and keys (F2 modal, /dpi-* commands, shortcut keys) ------------------------------------------

const until = async (pred) => { for (let i = 0; i < 60 && !pred(); i++) await new Promise((r) => setTimeout(r, 50)); assert.ok(pred(), "condition not reached"); };
const opt = (snap, id) => snap.options.find((o) => o.id === id);
const sent = (from) => state.requests.slice(from).filter((r) => !r.startsWith("GET "));
const mark = () => state.requests.length;
const setting = (s, settingId, value) => s.call({ type: "dpi_set_setting", settingId, value });

test("dpi_settings: info from the backend, options for model, thinking, read-only, prompts, extensions", async () => {
  state.ext = freshExt();
  resetAgentState();
  const s = await shim({ DPI_HUB_URL: "http://hub.example:4781" });
  try {
    const r = await s.call({ type: "dpi_settings" });
    assert.equal(r.success, true);
    const { info, options } = r.data;
    assert.deepEqual({ ...info, hints: undefined }, {
      device: "mini-host", session: "s1", sessionId: "s1", agent: "n", agentId: "1", cwd: "/w/proj", branch: "main",
      model: "m (p)", thinking: "low", readOnly: false, tools: ["read", "edit"], backendUrl: base, playgroundUrl: "http://hub.example:4781", hints: undefined,
    });
    assert.equal(info.context, undefined); // no assistant turn with usage yet: nothing honest to say
    assert.ok(info.hints.length >= 6 && info.hints.length <= 10 && info.hints.every((h) => h.length < 200));
    assert.ok(info.hints.some((h) => h.includes("keeps working on mini-host")));
    assert.ok(info.hints.some((h) => h.includes("http://hub.example:4781")));
    assert.deepEqual(options.map((o) => o.id), ["model", "thinking", "read_only", "prompt:smart_ass_opus_fix.md", "prompt:one_sentence.md", "ext:system-prompts"]);
    assert.deepEqual(opt(r.data, "model").values.map((v) => v.value), ["p/m", "p/big", "q/a/b:free"]);
    assert.equal(opt(r.data, "model").currentValue, "p/m");
    assert.equal(opt(r.data, "thinking").currentValue, "low");
    assert.deepEqual(opt(r.data, "thinking").values.map((v) => v.value), ["off", "minimal", "low", "medium", "high", "xhigh"]);
    assert.match(opt(r.data, "thinking").description, /does not reason/); // model m has reasoning:false
    assert.equal(opt(r.data, "read_only").type, "boolean");
    // a prompt is on only when its extension is too: the extension is off, so even the checked one reads off
    assert.equal(opt(r.data, "prompt:smart_ass_opus_fix.md").currentValue, false);
    assert.equal(opt(r.data, "prompt:smart_ass_opus_fix.md").category, "System prompts");
    assert.equal(opt(r.data, "ext:system-prompts").category, "Extensions");
    assert.equal(opt(r.data, "compaction"), undefined); // model m's 1000-token window fits no percentage
  } finally { s.close(); }
});

test("dpi_settings reads context usage from the last assistant turn, and offers compaction when the window allows", async () => {
  state.ext = freshExt();
  resetAgentState();
  state.model = { provider: "p", modelId: "big" };
  const s = await shim({}, "dpi://s1/4");
  try {
    const { data } = await s.call({ type: "dpi_settings" });
    assert.deepEqual(data.info.context, { tokens: 200, window: 200000, percent: 0 });
    assert.equal(data.info.playgroundUrl, null);
    const c = opt(data, "compaction");
    // reserve 20000 of 200000: a summary blocks the request at 90%
    assert.equal(c.currentValue, "90");
    assert.deepEqual(c.values.map((v) => v.value), ["off", "50", "60", "70", "80", "90"]);
    assert.equal(c.category, "Context");
  } finally { s.close(); resetAgentState(); }
});

test("dpi_set_setting: each id calls the endpoint the playground uses and returns a fresh snapshot", async () => {
  state.ext = freshExt();
  resetAgentState();
  const s = await shim();
  try {
    let at = mark();
    let r = await setting(s, "model", "p/big");
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/config {"model":{"provider":"p","modelId":"big"}}']);
    assert.equal(opt(r.data, "model").currentValue, "p/big");
    assert.equal(r.data.info.model, "big (p)");

    at = mark();
    r = await setting(s, "model", "q/a/b:free"); // provider/id where the id has a slash
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/config {"model":{"provider":"q","modelId":"a/b:free"}}']);
    await setting(s, "model", "p/big");

    at = mark();
    r = await setting(s, "thinking", "high");
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/config {"thinkingLevel":"high"}']);
    assert.equal(r.data.info.thinking, "high");

    at = mark();
    r = await setting(s, "read_only", true);
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/config {"readOnly":true}']);
    assert.equal(r.data.info.readOnly, true);
    assert.deepEqual(r.data.info.tools, ["read"]); // edit is no longer offered

    at = mark();
    r = await setting(s, "ext:system-prompts", true);
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/extensions {"name":"system-prompts","enabled":true}']);
    assert.equal(opt(r.data, "ext:system-prompts").currentValue, true);
    assert.equal(opt(r.data, "prompt:smart_ass_opus_fix.md").currentValue, true);

    at = mark();
    r = await setting(s, "prompt:one_sentence.md", true);
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/extensions {"prompt":"one_sentence.md","enabled":true}']);
    assert.equal(opt(r.data, "prompt:one_sentence.md").currentValue, true);

    at = mark();
    r = await setting(s, "prompt:one_sentence.md", false);
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/1/extensions {"prompt":"one_sentence.md","enabled":false}']);
    assert.equal(opt(r.data, "prompt:one_sentence.md").currentValue, false);

    at = mark();
    r = await setting(s, "compaction", "70");
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/compaction {"enabled":true,"reserveTokens":60000}']);
    assert.equal(opt(r.data, "compaction").currentValue, "70");

    at = mark();
    r = await setting(s, "compaction", "off");
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/compaction {"enabled":false}']);
    assert.equal(opt(r.data, "compaction").currentValue, "off");
  } finally { s.close(); resetAgentState(); }
});

test("turning a saved prompt on while its extension is off also turns the extension on", async () => {
  state.ext = freshExt();
  const s = await shim();
  try {
    const at = mark();
    await setting(s, "prompt:one_sentence.md", true);
    assert.deepEqual(sent(at), [
      'PATCH /sessions/s1/agents/1/extensions {"prompt":"one_sentence.md","enabled":true}',
      'PATCH /sessions/s1/agents/1/extensions {"name":"system-prompts","enabled":true}',
    ]);
  } finally { s.close(); }
});

test("dpi_set_setting errors come back as success:false with a message, change nothing, and the shim keeps answering", async () => {
  state.ext = freshExt();
  resetAgentState();
  const s = await shim();
  try {
    const at = mark();
    const bad = async (id, value, pattern) => {
      const r = await setting(s, id, value);
      assert.equal(r.success, false, `${id}=${value}`);
      assert.match(r.error, pattern);
    };
    await bad("nope", true, /unknown setting "nope"/);
    await bad("thinking", "wild", /thinking is one of off, minimal/);
    await bad("read_only", "yes", /Read-only is on or off/);
    await bad("model", "p/missing", /no model "p\/missing"/);
    await bad("prompt:nope.md", true, /no saved system prompt nope\.md/);
    await bad("compaction", "50", /50% of this model's 1k window leaves no room/); // model m: 1000-token window
    await bad("compaction", "soon", /not a compaction setting/);
    assert.deepEqual(sent(at), []);
    state.failConfig = true; // the backend itself says no
    await bad("thinking", "high", /^Invalid thinking level$/);
    state.failConfig = false;
    assert.equal((await setting(s, "thinking", "high")).success, true);
    assert.equal((await s.call({ type: "dpi_settings" })).success, true);
  } finally { s.close(); resetAgentState(); }
});

test("settings helpers: options omit what they cannot source, compaction maps percent to reserve tokens", () => {
  const agent = { config: { model: { provider: "z", modelId: "unlisted" } } };
  const out = buildOptions({ agent, device: { models: [] }, ext: { extensions: [] }, compaction: undefined });
  assert.deepEqual(out.map((o) => o.id), ["model", "thinking", "read_only"]);
  assert.deepEqual(out[0].values, [{ value: "z/unlisted", name: "unlisted", description: "z" }]); // a model the list lacks is still shown
  assert.equal(out[1].currentValue, "medium");
  assert.deepEqual(compactionPatch(272000, "80"), { enabled: true, reserveTokens: 54400 });
  assert.deepEqual(compactionPatch(272000, "off"), { enabled: false });
  assert.throws(() => compactionPatch(0, "80"), /window is unknown/);
  assert.throws(() => compactionPatch(272000, "100"), /not a compaction setting/);
  // a custom threshold already in effect is offered as the current value, beside the standard ones
  const device = { models: [{ provider: "p", id: "m", contextWindow: 272000 }] };
  const custom = buildOptions({ agent: { config: { model: { provider: "p", modelId: "m" } } }, device, ext: { extensions: [] }, compaction: { enabled: true, reserveTokens: 16384 } });
  const c = custom.find((o) => o.id === "compaction");
  assert.equal(c.currentValue, "94");
  assert.deepEqual(c.values.map((v) => v.value), ["off", "50", "60", "70", "80", "90", "94"]);
});

test("playground URL: env, then the client file for this backend, then :4781 on the same host if it answers; Prime has none", async () => {
  const never = async () => { throw new Error("must not probe"); };
  const url = (over) => resolvePlaygroundUrl({ env: {}, clientEnv: {}, base: "http://100.66.99.64:4780", probe: never, ...over });
  assert.equal(await url({ env: { DPI_HUB_URL: "http://env-hub:4781/" }, clientEnv: { DPI_HUB_URL: "http://file:1" } }), "http://env-hub:4781");
  assert.equal(await url({ clientEnv: { DPI_URL: "http://100.66.99.64:4780", DPI_HUB_URL: "http://file-hub:4781" } }), "http://file-hub:4781");
  assert.equal(await url({ clientEnv: { DPI_HUB_URL: "http://file-hub:4781" } }), "http://file-hub:4781"); // a file that names no backend
  // the file is for another backend: its hub is not this one's
  const probed = [];
  const probe = async (u) => (probed.push(u), true);
  assert.equal(await url({ clientEnv: { DPI_URL: "http://elsewhere:4780", DPI_HUB_URL: "http://file-hub:4781" }, probe }), "http://100.66.99.64:4781");
  assert.deepEqual(probed, ["http://100.66.99.64:4781"]);
  assert.equal(await url({ probe: async () => false }), null);
  assert.equal(await url({ base: "http://100.66.99.64:4782", clientEnv: { DPI_HUB_URL: "http://file-hub:4781" } }), null); // Prime backend
  assert.equal(await url({ base: "http://100.66.99.64:4782", env: { DPI_HUB_URL: "http://env-hub:4781" } }), "http://env-hub:4781"); // but an explicit one wins
});

test("the playground URL is read from the user's dpi client file when the environment has none, and the file is never echoed", async () => {
  const home = mkdtempSync(join(tmpdir(), "dpi-rpc-"));
  mkdirSync(join(home, ".dpi"));
  writeFileSync(join(home, ".dpi", "client.env"), "DPI_FAKE_SECRET=not-for-output-xyz\nDPI_HUB_URL=http://filehub:4781\n");
  state.ext = freshExt();
  const s = await shim({ HOME: home });
  try {
    const { data } = await s.call({ type: "dpi_settings" });
    assert.equal(data.info.playgroundUrl, "http://filehub:4781");
    assert.ok(!JSON.stringify(s.lines).includes("not-for-output-xyz"));
  } finally { s.close(); }
});

test("the shortcut catalog goes out at startup in the pager's format, with keys that do not collide", async () => {
  const s = await shim();
  try {
    const w = await s.until((l) => l.method === "setWidget" && l.widgetKey === "__pi_extension_shortcuts__");
    assert.equal(w.widgetLines.length, 1);
    const { shortcuts } = JSON.parse(w.widgetLines[0]);
    assert.deepEqual(shortcuts.map((x) => x.key), ["alt+c", "alt+a", "alt+k", "alt+x", "alt+o", "alt+h"]);
    assert.ok(shortcuts.every((x) => x.extension === "dpi" && x.enabled === true && x.description && x.key === x.key.toLowerCase()));
    assert.equal(new Set(shortcuts.map((x) => x.key)).size, shortcuts.length);
    assert.deepEqual(shortcuts.map((x) => x.label), ["compact", "new agent", "fork", "stop", "playground", "help"]);
    assert.ok(shortcuts.every((x) => x.label.length <= 12 && x.label !== x.description));
    // none is an Option dead key (e i n u) or a word-motion letter (b f d): the terminal would never deliver those as Alt+letter
    assert.ok(shortcuts.every((x) => !/^alt\+[einubfd]$/.test(x.key)));
    // every key maps to a command the shim really has
    const names = (await s.call({ type: "get_commands" })).data.commands.map((c) => c.name);
    for (const k of SHORTCUTS) assert.ok(names.includes(k.command), k.command);
    // the catalog is sent again once the adapter asks for commands (it drops one that arrives before the session exists)
    await s.until((l) => l !== w && l.method === "setWidget" && l.widgetKey === "__pi_extension_shortcuts__");
  } finally { s.close(); }
});

test("/__pi_shortcut_dispatch <key> runs the mapped action, whatever the key's case; an unknown key says so", async () => {
  resetAgentState();
  const s = await shim();
  try {
    let at = mark();
    assert.equal((await s.call({ type: "prompt", message: "/__pi_shortcut_dispatch alt+c" })).success, true);
    await s.until((l) => l.method === "notify" && l.message.startsWith("Compacting this agent's context (task #42)"));
    assert.deepEqual(sent(at), ["POST /sessions/s1/agents/1/compact {}"]);
    at = mark();
    await s.call({ type: "prompt", message: "/__pi_shortcut_dispatch ALT+X" });
    await s.until((l) => l.method === "notify" && l.message.startsWith("Stopped"));
    assert.deepEqual(sent(at), ['POST /sessions/s1/agents/1/abort {"background":true}']);
    await s.call({ type: "prompt", message: "/__pi_shortcut_dispatch alt+z" });
    await s.until((l) => l.method === "notify" && l.notifyType === "error" && l.message === "/__pi_shortcut_dispatch: no Durable Pi action on alt+z");
  } finally { s.close(); }
});

test("/dpi-compact passes instructions; /dpi-rename renames the session for the main agent and the agent otherwise", async () => {
  const s = await shim();
  try {
    let at = mark();
    await s.call({ type: "prompt", message: "/dpi-compact keep the schema decisions" });
    await s.until((l) => l.method === "notify" && l.message.includes("task #42"));
    assert.deepEqual(sent(at), ['POST /sessions/s1/agents/1/compact {"instructions":"keep the schema decisions"}']);
    at = mark();
    await s.call({ type: "prompt", message: "/dpi-rename Deep work" });
    await s.until((l) => l.method === "notify" && l.message === "Session renamed to Deep work.");
    assert.deepEqual(sent(at), ['PATCH /sessions/s1 {"name":"Deep work"}']);
    await s.call({ type: "prompt", message: "/dpi-rename" });
    await s.until((l) => l.method === "notify" && l.notifyType === "error" && l.message === "/dpi-rename: usage: /dpi-rename <name>");
  } finally { s.close(); }
  const fork = await shim({}, "dpi://s1/2");
  try {
    const at = mark();
    await fork.call({ type: "prompt", message: "/dpi-rename Spike" });
    await fork.until((l) => l.method === "notify" && l.message === "Agent renamed to Spike.");
    assert.deepEqual(sent(at), ['PATCH /sessions/s1/agents/2 {"name":"Spike"}']);
  } finally { fork.close(); }
});

test("/dpi-fork branches at the last assistant reply; /dpi-new-agent adds a sibling; neither moves this window", async () => {
  const s = await shim();
  try {
    let at = mark();
    await s.call({ type: "prompt", message: "/dpi-fork experiment" });
    await s.until((l) => l.method === "notify" && l.message.startsWith("Forked at reply #6 as agent experiment."));
    assert.deepEqual(sent(at), ['POST /sessions/s1/agents/1/fork {"entryId":6,"name":"experiment"}']);
    at = mark();
    await s.call({ type: "prompt", message: "/dpi-new-agent Scratch" });
    await s.until((l) => l.method === "notify" && l.message.startsWith("New agent"));
    assert.deepEqual(sent(at), ['POST /sessions/s1/agents {"from":"1","name":"Scratch"}']);
  } finally { s.close(); }
  const empty = await shim({}, "dpi://s1/3"); // agent 3 never replied
  try {
    await empty.call({ type: "prompt", message: "/dpi-fork" });
    await empty.until((l) => l.method === "notify" && l.notifyType === "error" && l.message === "/dpi-fork: nothing to fork yet: the agent has not replied");
  } finally { empty.close(); }
});

test("/dpi-reset and /dpi-archive ask first: nothing happens until Yes, and No or dismissing does nothing", async () => {
  const s = await shim();
  try {
    let at = mark();
    await s.call({ type: "prompt", message: "/dpi-reset handing over" });
    const dialog = await s.until((l) => l.method === "confirm" && l.title === "Reset this agent?");
    assert.match(dialog.message, /history stays stored/);
    assert.ok(dialog.timeout > 0);
    await new Promise((r) => setTimeout(r, 150));
    assert.deepEqual(sent(at), []); // asked, not done
    s.send({ type: "extension_ui_response", id: dialog.id, confirmed: true });
    await s.until((l) => l.method === "notify" && l.message === "New context started.");
    assert.deepEqual(sent(at), ['POST /sessions/s1/agents/1/reset {"note":"handing over"}']);

    at = mark();
    const nothingDone = () => s.lines.filter((x) => x.method === "notify" && x.message === "Archive this session?: nothing done.").length;
    await s.call({ type: "prompt", message: "/dpi-archive" });
    const no = await s.until((l) => l.method === "confirm" && l.title === "Archive this session?");
    s.send({ type: "extension_ui_response", id: no.id, confirmed: false });
    await until(() => nothingDone() === 1);
    await s.call({ type: "prompt", message: "/dpi-archive" });
    const dismissed = await s.until((l) => l.method === "confirm" && l.id !== no.id && l.title === "Archive this session?");
    s.send({ type: "extension_ui_response", id: dismissed.id, cancelled: true }); // dismissed, not answered
    await until(() => nothingDone() === 2);
    assert.deepEqual(sent(at), []);

    await s.call({ type: "prompt", message: "/dpi-archive" });
    const yes = await s.until((l) => l.method === "confirm" && l.id !== no.id && l.id !== dismissed.id && l.title === "Archive this session?");
    s.send({ type: "extension_ui_response", id: "not-ours", confirmed: true }); // an id we never opened changes nothing
    await new Promise((r) => setTimeout(r, 100));
    assert.deepEqual(sent(at), []);
    s.send({ type: "extension_ui_response", id: yes.id, confirmed: true });
    await s.until((l) => l.method === "notify" && l.message.startsWith("Session archived."));
    assert.deepEqual(sent(at), ["DELETE /sessions/s1"]);
    s.send({ type: "extension_ui_response", id: yes.id, confirmed: true }); // answered once only
    await new Promise((r) => setTimeout(r, 100));
    assert.deepEqual(sent(at), ["DELETE /sessions/s1"]);
  } finally { s.close(); }
});

test("a confirmed action the backend refuses says why instead of going quiet", async () => {
  state.failDelete = true;
  const s = await shim();
  try {
    await s.call({ type: "prompt", message: "/dpi-archive" });
    const d = await s.until((l) => l.method === "confirm" && l.title === "Archive this session?");
    s.send({ type: "extension_ui_response", id: d.id, confirmed: true });
    await s.until((l) => l.method === "notify" && l.notifyType === "error" && l.message === "Archive this session?: session is busy");
    assert.ok(!s.lines.some((l) => l.method === "notify" && l.message?.startsWith("Session archived")));
  } finally { s.close(); state.failDelete = false; }
});

test("/dpi-help, /dpi-settings and /dpi-open say something useful", async () => {
  state.ext = freshExt();
  resetAgentState();
  const s = await shim({ DPI_HUB_URL: "http://hub.example:4781" });
  try {
    await s.call({ type: "prompt", message: "/dpi-help" });
    const help = await s.until((l) => l.method === "dpiHelp");
    assert.ok(help.intro.startsWith("Durable Pi:") && help.intro.includes("mini-host"));
    assert.deepEqual(help.sections.map((x) => x.title), ["Keys", "Commands"]);
    const [keys, commands] = help.sections.map((x) => x.rows.map(([key]) => key));
    for (const key of ["F2", "ctrl+r", "Alt+C"]) assert.ok(keys.includes(key), key);
    for (const command of ["/dpi-compact [instructions]", "/dpi-archive", "/dpi-help"]) assert.ok(commands.includes(command), command);
    assert.ok(help.sections.every((x) => x.rows.every((row) => row.length === 2 && row.every((cell) => typeof cell === "string" && cell))));
    assert.equal(help.playgroundUrl, "http://hub.example:4781");
    assert.ok(!JSON.stringify(help).includes("__pi_")); // internal bridge commands stay out of the guide
    await s.call({ type: "prompt", message: "/dpi-open" });
    await s.until((l) => l.method === "notify" && l.message === "Playground: http://hub.example:4781");
    await s.call({ type: "prompt", message: "/dpi-settings" });
    const sum = await s.until((l) => l.method === "notify" && l.message.startsWith("Press F2"));
    assert.equal(sum.message, "Press F2 to change these. Now: m (p) · thinking low · read-only off · 0 prompts and extensions on");
  } finally { s.close(); }
  const none = await shim();
  try {
    await none.call({ type: "prompt", message: "/dpi-open" });
    await none.until((l) => l.method === "notify" && l.message === "This backend has no playground.");
    await none.call({ type: "prompt", message: "/dpi-help" });
    assert.equal((await none.until((l) => l.method === "dpiHelp")).playgroundUrl, null);
  } finally { none.close(); }
});
