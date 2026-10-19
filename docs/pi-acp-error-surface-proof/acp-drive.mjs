// Drive an ACP agent over stdio: initialize -> session/new -> session/prompt. Raw transcript to argv[2].
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
const [cmd, out, ...args] = process.argv.slice(2);
const proc = spawn(cmd, args, { stdio: ["pipe", "pipe", "pipe"], cwd: "/tmp" });
const log = []; let buf = ""; let id = 0; const pending = new Map();
const send = (method, params) => new Promise((res) => { const i = ++id; pending.set(i, res); const l = JSON.stringify({ jsonrpc: "2.0", id: i, method, params }); log.push(`>>> ${l}`); proc.stdin.write(l + "\n"); });
proc.stderr.on("data", (d) => log.push(`[stderr] ${String(d).trimEnd()}`));
proc.stdout.on("data", (d) => { buf += d; let n; while ((n = buf.indexOf("\n")) >= 0) { const l = buf.slice(0, n); buf = buf.slice(n + 1); if (!l.trim()) continue; log.push(`<<< ${l}`); let m; try { m = JSON.parse(l); } catch { continue; }
  if (m.id !== undefined && m.method === undefined) pending.get(m.id)?.(m);
  else if (m.method === "session/request_permission") proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: m.id, result: { outcome: { outcome: "selected", optionId: m.params.options?.[0]?.id } } }) + "\n"); } });
const timer = setTimeout(() => { log.push("!!! timeout"); finish(2); }, 90000);
function finish(code) { clearTimeout(timer); writeFileSync(out, log.join("\n") + "\n"); proc.kill(); process.exit(code); }
const init = await send("initialize", { protocolVersion: 1, clientCapabilities: { fs: { readTextFile: false, writeTextFile: false } } });
const ns = await send("session/new", { cwd: "/tmp", mcpServers: [] });
const sid = ns.result?.sessionId; if (!sid) { log.push("!!! no sessionId"); finish(1); }
const pr = await send("session/prompt", { sessionId: sid, prompt: [{ type: "text", text: "Say hello in one word." }] });
console.log("prompt response:", JSON.stringify(pr.result ?? pr.error));
const chunks = log.filter((l) => l.startsWith("<<<") && l.includes("agent_message_chunk")).map((l) => { try { return JSON.parse(l.slice(4)).params.update.content.text; } catch { return null; } }).filter(Boolean);
console.log("agent_message_chunk texts:", JSON.stringify(chunks));
finish(0);
