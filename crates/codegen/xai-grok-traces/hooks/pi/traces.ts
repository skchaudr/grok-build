// grok-traces hook for Pi: tells the session index which file changed. No network, no env loading.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"
import { spawn } from "node:child_process"
import { homedir } from "node:os"
import { join } from "node:path"

const SOURCE = "pi"
const BIN = join(homedir(), ".grok", "bin", "grok-traces")

function emit(event: string, ctx: any, tool = "", toolUseId = "") {
  try {
    const sm = ctx?.sessionManager
    const body = JSON.stringify({
      event, tool, tool_use_id: toolUseId, cwd: ctx?.cwd ?? process.cwd(),
      session_id: String(sm?.getSessionId?.() ?? ""), session_file: String(sm?.getSessionFile?.() ?? ""),
      harness_pid: process.pid,
    })
    const child = spawn(BIN, ["hook", SOURCE], { stdio: ["pipe", "ignore", "ignore"], detached: true })
    child.on("error", () => {})
    child.stdin?.on("error", () => {})
    child.stdin?.end(body)
    child.unref()
  } catch {}
}

export default function tracesHook(pi: ExtensionAPI): void {
  for (const ev of ["session_start", "agent_start", "agent_end", "session_shutdown"] as const) {
    pi.on(ev, (_e: any, ctx: any) => { emit(ev, ctx) })
  }
  for (const ev of ["tool_execution_start", "tool_execution_end"] as const) {
    pi.on(ev, (e: any, ctx: any) => { emit(ev, ctx, String(e?.toolName ?? ""), String(e?.toolCallId ?? "")) })
  }
}
