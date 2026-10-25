// bb plugin: on thread lifecycle events, spawn `~/.grok/bin/grok-traces hook bb` detached with
// {"event","thread_id"} on stdin. Per-thread throttle (leading + one trailing per 1s). Never awaits, never throws.
// Install: `hooks/install.sh --apply` (copies to ~/.grok/hooks/bb, runs `bb plugin install`). No dependencies.
import { spawn } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

type Handler = (p: { thread?: { id?: string } }) => void | Promise<void>;
interface Api {
  events: { on(name: string, h: Handler): unknown };
  onDispose?(f: () => void): void;
  log?: { warn(m: string): void };
}

const BIN = join(homedir(), ".grok", "bin", "grok-traces");
const THROTTLE_MS = 1000;
const EVENTS = ["thread.created", "thread.active", "thread.idle", "thread.failed", "thread.archived", "thread.deleted"];

export default async function plugin(bb: Api) {
  const state = new Map<string, { at: number; timer?: ReturnType<typeof setTimeout> }>();

  function emit(threadId: string, event: string) {
    try {
      const child = spawn(BIN, ["hook", "bb"], { stdio: ["pipe", "ignore", "ignore"], detached: true });
      child.on("error", () => {});
      child.stdin?.on("error", () => {});
      child.stdin?.end(JSON.stringify({ event, thread_id: threadId }));
      child.unref();
    } catch {}
  }

  for (const name of EVENTS) {
    bb.events.on(name, ({ thread }) => {
      try {
        const id = thread?.id;
        if (!id) return;
        const s = state.get(id) ?? { at: 0 };
        state.set(id, s);
        const wait = s.at + THROTTLE_MS - Date.now();
        if (wait <= 0 && !s.timer) { s.at = Date.now(); return emit(id, name); }
        if (s.timer) return;
        s.timer = setTimeout(() => { s.timer = undefined; s.at = Date.now(); emit(id, name); }, Math.max(wait, 0));
        s.timer.unref?.();
      } catch {}
    });
  }
  bb.onDispose?.(() => { for (const s of state.values()) if (s.timer) clearTimeout(s.timer); });
}
