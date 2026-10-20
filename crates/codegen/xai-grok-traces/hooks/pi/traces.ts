// grok-traces hook for Pi: tells the session index which file changed. No network, no env loading.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"
import { spawn } from "node:child_process"
import { homedir } from "node:os"
import { join } from "node:path"

const SOURCE = "pi"
const BIN = join(homedir(), ".grok", "bin", "grok-traces")
const DEBOUNCE_MS = 500
const last = new Map<string, { at: number; timer?: ReturnType<typeof setTimeout> }>()

function emit(event: string, ctx: any, tool = "") {
  try {
    const sm = ctx?.sessionManager
    const body = JSON.stringify({
      event, tool, cwd: ctx?.cwd ?? process.cwd(),
      session_id: String(sm?.getSessionId?.() ?? ""), session_file: String(sm?.getSessionFile?.() ?? ""),
    })
    const child = spawn(BIN, ["hook", SOURCE], { stdio: ["pipe", "ignore", "ignore"], detached: true })
    child.on("error", () => {})
    child.stdin?.on("error", () => {})
    child.stdin?.end(body)
    child.unref()
  } catch {}
}

// tool_result: leading spawn, then at most one trailing spawn per window so the last event is not lost.
function debounced(event: string, ctx: any, tool: string) {
  try {
    const key = String(ctx?.sessionManager?.getSessionId?.() ?? "")
    const s = last.get(key) ?? { at: 0 }
    last.set(key, s)
    const wait = s.at + DEBOUNCE_MS - Date.now()
    if (wait <= 0 && !s.timer) { s.at = Date.now(); return emit(event, ctx, tool) }
    if (s.timer) return
    s.timer = setTimeout(() => { s.timer = undefined; s.at = Date.now(); emit(event, ctx, tool) }, Math.max(wait, 0))
    s.timer.unref?.()
  } catch {}
}

export default function tracesHook(pi: ExtensionAPI): void {
  for (const ev of ["session_start", "turn_end", "agent_end"] as const) {
    pi.on(ev, (_e: any, ctx: any) => { emit(ev, ctx) })
  }
  pi.on("tool_result", (e: any, ctx: any) => { debounced("tool_result", ctx, String(e?.toolName ?? "")) })
  pi.on("session_shutdown", (_e: any, ctx: any) => {
    try {
      const k = String(ctx?.sessionManager?.getSessionId?.() ?? "")
      clearTimeout(last.get(k)?.timer)
      last.delete(k)
    } catch {}
    emit("session_shutdown", ctx)
  })
}
