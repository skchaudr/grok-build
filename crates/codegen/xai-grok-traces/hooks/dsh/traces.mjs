// grok-traces hook for DSH (Cordis plugin). Mount from a profile's cordis.patch.yml:
//   - insert:
//     - id: grok-traces
//       name: '/abs/path/to/traces.mjs'
// Observes the durable `session/event` firehose; spawns `grok-traces hook dsh` detached, at most once per 1s per session
// (leading + one trailing). No network, no env loading, errors swallowed.
// Verified: API shape read from ~/.dsh/plugins/jev-guard.mjs and profiles/web/moshi-bridge.mjs; log path/name from the
// session-persistence-jsonl package (live logs here are `session.jsonl.zstd`, later generations `session.vN.jsonl.zstd`).
// Not run inside a live DSH; the spawn/throttle/path logic is exercised by hooks/test.sh with a stub host.
import { spawn } from 'node:child_process'
import { existsSync, readdirSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

const BIN = join(homedir(), '.grok', 'bin', 'grok-traces')
const ROOT = join(homedir(), '.dsh', 'sessions')
const THROTTLE_MS = 1000
const LOG = /^session(\.v(\d+))?\.jsonl\.zstd$/
const state = new Map() // session id -> { at, timer, file }

// <root>/<project-key>/<encoded id>/session[.vN].jsonl.zstd; the project key is not derivable here, so scan the few project dirs.
function findFile(id) {
  try {
    for (const proj of readdirSync(ROOT)) {
      const dir = join(ROOT, proj, id)
      if (!existsSync(dir)) continue
      const logs = readdirSync(dir).map(n => [n, LOG.exec(n)]).filter(([, m]) => m)
      logs.sort((a, b) => Number(b[1][2] ?? 0) - Number(a[1][2] ?? 0))
      if (logs[0]) return join(dir, logs[0][0])
    }
  } catch {}
  return ''
}

function emit(id, s, event, tool, cwd) {
  try {
    s.file ||= findFile(id) // re-tried each emit until the log exists
    const child = spawn(BIN, ['hook', 'dsh'], { stdio: ['pipe', 'ignore', 'ignore'], detached: true })
    child.on('error', () => {})
    child.stdin?.on('error', () => {})
    child.stdin?.end(JSON.stringify({ event, tool, cwd, session_id: id, session_file: s.file }))
    child.unref()
  } catch {}
}

export default {
  name: 'grok-traces',
  apply(ctx) {
    ctx.on('session/event', (session, event) => {
      try {
        const id = session?.id
        if (!id || typeof event?.type !== 'string') return
        const s = state.get(id) ?? { at: 0, file: '' }
        state.set(id, s)
        const tool = event.type.startsWith('tool/') && typeof event.data?.name === 'string' ? event.data.name : ''
        const cwd = session?.header?.cwd ?? process.cwd()
        const wait = s.at + THROTTLE_MS - Date.now()
        if (wait <= 0 && !s.timer) { s.at = Date.now(); return emit(id, s, event.type, tool, cwd) }
        if (s.timer) return
        s.timer = setTimeout(() => { s.timer = undefined; s.at = Date.now(); emit(id, s, event.type, tool, cwd) }, Math.max(wait, 0))
        s.timer.unref?.()
      } catch {}
    }, { global: true })
  },
}
