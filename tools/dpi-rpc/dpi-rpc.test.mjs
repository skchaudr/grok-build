// node --test tools/dpi-rpc/ : drives dpi-rpc.mjs over stdin/stdout against a fake dpi HTTP server.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";

const SHIM = join(dirname(fileURLToPath(import.meta.url)), "dpi-rpc.mjs");
const TOKEN = "test-token";
const msg = (role, ts, content = "x") => ({ role, content, timestamp: ts });
const agents1 = [
  { id: 1, name: "Main", kind: "root", busy: false },
  { id: 2, name: "Alt", kind: "fork", parentId: 1, busy: true },
  { id: 3, name: "Sub", kind: "subagent", ownerAgentId: 2, busy: false },
];
const state = { tasks: [], prime: null, requests: [] };
const entries = {
  "s1/1": [
    { id: 4, kind: "pi.system", model: [msg("system", 1500)] },
    { id: 5, kind: "pi.user", model: [msg("user", 2000, "first question")] },
    { id: 6, kind: "pi.assistant", model: [msg("assistant", 3000)] },
    { id: 7, kind: "pi.user", model: [msg("user", 4000, [{ type: "text", text: "second" }])] },
  ],
  "s1/3": [{ id: 8, kind: "pi.user", model: [msg("user", 5000)] }],
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
    if (p === "/device") return json(res, 200, { models: [{ provider: "p", id: "m", contextWindow: 1000 }] });
    if (p === "/sessions" && req.method === "GET") {
      return json(res, 200, [
        { id: "s1", name: "one", cwd: "/w", model: "m", createdAt: 1000, agents: agents1 },
        { id: "s2", name: "two", cwd: "/w", model: "m", createdAt: 9000, agents: [{ id: 1, name: "Main", kind: "root", busy: false }] },
      ]);
    }
    if (p === "/sessions" && req.method === "POST") return json(res, 201, { id: "s3", agents: [{ id: 1 }] });
    if (p === "/sessions/s1/agents" && req.method === "POST") return json(res, 201, { id: 10 });
    if ((m = p.match(/^\/sessions\/(s\d)$/))) return json(res, 200, { id: m[1], name: m[1], cwd: "/w" });
    if (p.endsWith("/tasks")) return json(res, 200, state.tasks);
    if (p.endsWith("/prime/work")) return state.prime ? json(res, 200, state.prime) : json(res, 404, { error: { message: "no such route" } });
    if ((m = p.match(/^\/sessions\/(s\d)\/agents\/(\w+)(\/.*)?$/))) {
      const [, s, a, rest = ""] = m;
      const id = a === "main" ? 1 : Number(a);
      if (rest === "/events") {
        res.writeHead(200, { "content-type": "text/event-stream" });
        return res.write(`data: ${JSON.stringify({ type: "snapshot", snapshot: { entries: [] } })}\n\n`);
      }
      if (rest === "/entries") return entries[`${s}/${id}`] ? json(res, 200, { items: entries[`${s}/${id}`] }) : json(res, 500, { error: { message: "boom" } });
      if (rest === "/fork") return json(res, 201, { id: 9 });
      if (rest === "") return json(res, 200, { id, sessionId: s, name: "n", kind: "root", busy: false, config: { model: { provider: "p", modelId: "m" }, cwd: "/w/proj", thinkingLevel: "low" } });
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
const deploy = (over = {}) => ({ id: 1, incidentId: "INC-0001", status: "awaiting_approval", commit: "abc123", branch: "fix/x", summary: "retry fix", diffStat: "1 file changed", ...over });
const ops = { deploys: [], decisions: [] };
const opsServer = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (req.method === "GET" && req.url === "/api/deploys") return json(res, 200, ops.deploys);
    const m = req.url.match(/^\/api\/deploys\/(\d+)\/decide$/);
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
    assert.equal(q.message, "Deploy INC-0001: retry fix\ncommit abc123\nbranch fix/x\n1 file changed");
    await s.status("ops-deploy", "1 deploy awaiting approval");
    await settle();
    assert.equal(s.lines.filter(isConfirm).length, 1); // polling again does not re-prompt
    s.send({ type: "extension_ui_response", id: q.id, confirmed: true });
    await s.until(() => ops.decisions.length > 0);
    assert.deepEqual(ops.decisions, [[1, { approved: true }]]);
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

test("no dialog and no noise when ops is down", async () => {
  const s = await shim({ DPI_OPS_URL: "http://127.0.0.1:1" });
  try {
    await s.call({ type: "get_state" });
    await settle();
    assert.equal(s.lines.filter((l) => isConfirm(l) || l.statusKey === "ops-deploy").length, 0);
  } finally { s.close(); }
});

test("a deploy decided elsewhere withdraws the prompt: the late answer is ignored", async () => {
  Object.assign(ops, { deploys: [deploy({ id: 3 })], decisions: [] });
  const s = await shim({ DPI_OPS_URL: opsUrl });
  try {
    const q = await s.until(isConfirm);
    ops.deploys = [deploy({ id: 3, status: "approved" })];
    await s.until((l) => l.method === "notify" && /decided elsewhere/.test(l.message));
    s.send({ type: "extension_ui_response", id: q.id, confirmed: true });
    await settle();
    assert.deepEqual(ops.decisions, []);
  } finally { s.close(); }
});
