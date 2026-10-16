#!/usr/bin/env node
// Fake ACP agent for scripts/acp-replay.sh.
// Answers initialize and session/new, then on session/prompt streams every
// session/update notification from the jsonl file, then idles on stdin.
//
// The pager drops session/update lines whose sessionId is not the live
// session it created. Every streamed params.sessionId is rewritten to that
// id. Codemode captures use one session, so this is a rename. A recording
// that split text across child session ids (the subagent fixture) lands in
// the one pane so the capture can show it.
//
// Stdout is the ACP channel. Diagnostics go to ACP_REPLAY_LOG when set.

import fs from "node:fs";
import readline from "node:readline";

const file = process.argv[2];
if (!file) {
  process.stderr.write("usage: acp-fake-peer.mjs <acp.jsonl>\n");
  process.exit(2);
}

const delayMs = Number(process.env.ACP_REPLAY_DELAY_MS ?? "15");
const doneFile = process.env.ACP_REPLAY_DONE || "";
const logFile = process.env.ACP_REPLAY_LOG || "";

function log(message) {
  if (!logFile) return;
  fs.appendFileSync(logFile, message + "\n");
}

function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function result(id, payload) {
  emit({ jsonrpc: "2.0", id, result: payload });
}

const updates = [];
for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
  if (!line.trim()) continue;
  let msg;
  try {
    msg = JSON.parse(line);
  } catch (err) {
    log(`skip non-json: ${err.message}`);
    continue;
  }
  if (msg.method === "session/update" && msg.params && msg.params.update) {
    updates.push(msg);
  }
}
log(`loaded ${updates.length} session/update lines from ${file}`);

let liveSid = null;
let streamed = false;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function streamThenFinish(id) {
  const sid = liveSid || "replay-session";
  liveSid = sid;
  for (const msg of updates) {
    const copy = structuredClone(msg);
    delete copy._capture;
    if (copy.params) copy.params.sessionId = sid;
    emit(copy);
    if (delayMs > 0) await sleep(delayMs);
  }
  result(id, { stopReason: "end_turn" });
  streamed = true;
  if (doneFile) fs.writeFileSync(doneFile, `updates=${updates.length}\n`);
  log(`streamed ${updates.length} updates as session ${sid}`);
}

function handle(line) {
  const raw = line.trim();
  if (!raw) return;
  let msg;
  try {
    msg = JSON.parse(raw);
  } catch (err) {
    log(`bad stdin json: ${err.message}`);
    return;
  }
  const method = msg.method;
  const id = msg.id;
  log(`recv ${method ?? "(no method)"} id=${id ?? "-"}`);
  if (method === "initialize" && id !== undefined) {
    result(id, {
      protocolVersion: 1,
      agentCapabilities: { loadSession: false },
      authMethods: [],
      agentInfo: { name: "acp-fake-peer", version: "0" },
    });
    return;
  }
  if (method === "session/new" && id !== undefined) {
    liveSid = "replay-session";
    result(id, { sessionId: liveSid });
    return;
  }
  if (method === "session/prompt" && id !== undefined) {
    if (streamed) {
      result(id, { stopReason: "end_turn" });
      return;
    }
    streamThenFinish(id).catch((err) => {
      log(`stream failed: ${err.stack || err.message}`);
      result(id, { stopReason: "end_turn" });
    });
    return;
  }
  if (id !== undefined) {
    result(id, {});
  }
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on("line", handle);
rl.on("close", () => process.exit(0));
