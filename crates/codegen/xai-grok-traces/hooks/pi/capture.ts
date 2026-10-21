// Step 0 payload capture for Pi (SPEC-live): pipes each event + process.pid to capture.sh. Local only.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"
import { spawn } from "node:child_process"
import { homedir } from "node:os"
import { join } from "node:path"

const CAPTURE = join(homedir(), ".grok", "bin", "traces-capture.sh")
const EVENTS = ["session_start", "agent_start", "tool_call", "tool_execution_start", "tool_execution_end", "tool_result", "turn_end", "agent_end", "session_shutdown"] as const

// Long strings (tool output, file bodies) are cut; only field names and shapes matter here.
const clip = (_k: string, v: unknown) => (typeof v === "string" && v.length > 300 ? v.slice(0, 300) + "…" : v)

export default function captureHook(pi: ExtensionAPI): void {
  for (const ev of EVENTS) {
    pi.on(ev as any, (e: any, ctx: any) => {
      try {
        const sm = ctx?.sessionManager
        const body = JSON.stringify({
          event: ev, pid: process.pid, session_id: String(sm?.getSessionId?.() ?? ""),
          session_file: String(sm?.getSessionFile?.() ?? ""), payload: e,
        }, clip)
        const child = spawn(CAPTURE, ["pi", ev], { stdio: ["pipe", "ignore", "ignore"], detached: true })
        child.on("error", () => {})
        child.stdin?.on("error", () => {})
        child.stdin?.end(body)
        child.unref()
      } catch {}
    })
  }
}
