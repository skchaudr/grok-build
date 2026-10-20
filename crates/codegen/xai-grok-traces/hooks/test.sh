#!/usr/bin/env bash
# Tests install.sh against a synthetic HOME. Never touches the real home.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
export HOME="$T/home"; mkdir -p "$HOME/.claude" "$HOME/.codex" "$HOME/.grok/bin" "$HOME/.grok/hooks" "$HOME/.omp/agent/extensions" "$HOME/.pi/agent"
fail=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fail=1; }
expect() { local want="$1" got="$2" msg="$3"; [ "$want" = "$got" ] && ok "$msg" || bad "$msg (want $want got $got)"; }

cat > "$HOME/.claude/settings.json" <<'J'
{
  "model": "x",
  "hooks": {
    "SessionStart": [
      {"matcher": "*", "hooks": [{"type": "command", "command": "bash \"$HOME/.claude/hooks/log-commands.sh\" SessionStart"}]},
      {"hooks": [{"type": "command", "command": "'/opt/homebrew/bin/moshi-hook' claude-hook", "async": true}]}
    ],
    "PostToolUse": [
      {"matcher": "Bash", "hooks": [{"type": "command", "command": "atuin hook claude-code"}, {"type": "command", "command": "keep-me.sh"}]},
      {"hooks": [{"type": "command", "command": "if [ -n \"$ORCA_TOKEN\" ]; then orca-hook; fi", "timeout": 10}]}
    ],
    "PreToolUse": [
      {"matcher": "*", "hooks": [{"type": "command", "command": "bash \"$HOME/.claude/hooks/log-commands.sh\" PreToolUse"}]}
    ]
  }
}
J
cat > "$HOME/.codex/hooks.json" <<'J'
{"hooks": {
  "PostToolUse": [{"matcher": "^Bash$", "hooks": [{"type": "command", "command": "atuin hook codex"}]},
                  {"hooks": [{"type": "command", "command": "'/opt/homebrew/bin/moshi-hook' codex-hook"}]}],
  "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "bash '/x/.codex/hooks/log-commands.sh' UserPromptSubmit"}]},
                       {"hooks": [{"type": "command", "command": "orca-codex-hook", "timeout": 10}]}]
}}
J
ln -s /nonexistent/pi-observability.ts "$HOME/.omp/agent/extensions/pi-observability.ts"
S="$HOME/.claude/settings.json"; C="$HOME/.codex/hooks.json"
cp "$S" "$T/S.orig"; cp "$C" "$T/C.orig"
cnt() { jq --arg p "$2" '[.. | .command? // empty | select(contains($p))] | length' "$1"; }

# dry-run writes nothing; apply refuses without the binary
"$HERE/install.sh" >/dev/null
cmp -s "$S" "$T/S.orig" && cmp -s "$C" "$T/C.orig" && [ ! -e "$HOME/.grok/hooks/traces.json" ] && ok "dry-run writes nothing" || bad "dry-run wrote"
"$HERE/install.sh" --apply >/dev/null 2>&1 && bad "apply without binary should refuse" || ok "apply refuses without binary"

printf '#!/bin/sh\nexit 0\n' > "$HOME/.grok/bin/grok-traces"; chmod +x "$HOME/.grok/bin/grok-traces"
"$HERE/install.sh" --apply >/dev/null
"$HERE/install.sh" --apply >/dev/null   # second apply must be a no-op

for f in "$S" "$C"; do
  n="$(basename "$(dirname "$f")")"
  expect 0 "$(cnt "$f" log-commands.sh)" "$n: log-commands gone"
  expect 1 "$(cnt "$f" 'atuin hook')" "$n: atuin untouched"
done
expect 1 "$(cnt "$S" moshi-hook)" "claude: moshi untouched"
expect 1 "$(cnt "$C" moshi-hook)" "codex: moshi untouched"
expect 1 "$(cnt "$S" ORCA_TOKEN)" "claude: orca untouched"
expect 1 "$(cnt "$C" orca-codex-hook)" "codex: orca untouched"
expect 1 "$(cnt "$S" keep-me.sh)" "claude: sibling hook in same group kept"
expect 8 "$(cnt "$S" 'grok-traces" hook claude')" "claude: one entry per event after two applies"
expect 7 "$(cnt "$C" 'grok-traces" hook codex')" "codex: one entry per event after two applies"
expect "" "$(jq -r '.hooks | to_entries[] | select(.key == "UserPromptSubmit" or .key == "Stop") | .value[] | select(.matcher) | .matcher' "$S")" "claude: no matcher on UserPromptSubmit/Stop"
expect x "$(jq -r .model "$S")" "claude: unrelated keys preserved"
[ -f "$HOME/.grok/hooks/traces.json" ] && jq empty "$HOME/.grok/hooks/traces.json" && ok "grok template installed" || bad "grok template"
cmp -s "$HERE/omp/traces.ts" "$HOME/.omp/agent/extensions/traces.ts" && ok "omp extension installed" || bad "omp extension"
cmp -s "$HERE/pi/traces.ts" "$HOME/.pi/agent/extensions/traces.ts" && ok "pi extension installed" || bad "pi extension"
[ -L "$HOME/.omp/agent/extensions/pi-observability.ts" ] && ok "pi-observability kept without opt-in" || bad "pi-observability removed without opt-in"
ls "$HOME"/.claude/settings.json.bak-traces-* >/dev/null 2>&1 && ok "backup written" || bad "no backup"

"$HERE/install.sh" --apply --retire-pi-observability >/dev/null
[ ! -e "$HOME/.omp/agent/extensions/pi-observability.ts" ] && [ ! -L "$HOME/.omp/agent/extensions/pi-observability.ts" ] && ok "pi-observability retired with opt-in" || bad "retire"

out="$("$HERE/install.sh" --check)"; grep -q 'claude : .*SessionStart' <<<"$out" && ok "check reports claude wired" || bad "check"

"$HERE/install.sh" --remove --retire-pi-observability >/dev/null
expect 0 "$(cnt "$S" grok-traces)" "claude: --remove leaves no grok-traces"
expect 0 "$(cnt "$C" grok-traces)" "codex: --remove leaves no grok-traces"
expect 1 "$(cnt "$S" moshi-hook)" "claude: moshi survives --remove"
expect 1 "$(cnt "$C" orca-codex-hook)" "codex: orca survives --remove"
[ ! -e "$HOME/.grok/hooks/traces.json" ] && [ ! -e "$HOME/.omp/agent/extensions/traces.ts" ] && [ ! -e "$HOME/.pi/agent/extensions/traces.ts" ] && ok "--remove deletes copied templates" || bad "templates remain"
[ -L "$HOME/.omp/agent/extensions/pi-observability.ts" ] && ok "pi-observability restored" || bad "pi-observability not restored"

# JS templates: spawn args, stdin payload, debounce (needs node >= 22 for type stripping; skipped otherwise)
if command -v node >/dev/null && node --experimental-strip-types -e 0 2>/dev/null; then
  printf '#!/bin/sh\necho "$* $(cat)" >> "%s/log"\n' "$T" > "$HOME/.grok/bin/grok-traces"
  mkdir -p "$HOME/.dsh/sessions/--p--/session-abc"; : > "$HOME/.dsh/sessions/--p--/session-abc/session.jsonl.zstd"
  cat > "$T/run.mjs" <<'JS'
const [kind, file] = process.argv.slice(2); const mod = (await import(file)).default; const h = {}
const wait = ms => new Promise(r => setTimeout(r, ms))
if (kind === 'dsh') {
  mod.apply({ on: (e, f) => { h[e] = f } }); const s = { id: 'session-abc', header: { cwd: '/w' } }
  for (let i = 0; i < 5; i++) h['session/event'](s, { type: 'tool/call', data: { name: 'bash' } })
} else {
  mod({ on: (e, f) => { h[e] = f } }); const c = { cwd: '/w', sessionManager: { getSessionId: () => 'S', getSessionFile: () => '/f' } }
  h.session_start({}, c); await wait(600)
  for (let i = 0; i < 5; i++) { h.tool_result({ toolName: 'bash' }, c); await wait(20) }
}
await wait(1500)
JS
  for k in omp pi dsh; do
    f="$HERE/$k/traces.ts"; [ "$k" = dsh ] && f="$HERE/dsh/traces.mjs"
    : > "$T/log"; node --experimental-strip-types --no-warnings "$T/run.mjs" "$k" "$f"
    grep -q "^hook $k " "$T/log" && ok "$k: spawns 'hook $k'" || bad "$k: no spawn"
    n="$(wc -l < "$T/log" | tr -d ' ')"
    case $k in dsh) want=2 ;; *) want=3 ;; esac   # leading + one trailing (pi/omp add session_start)
    expect $want "$n" "$k: burst debounced to leading + trailing"
  done
  grep -q 'session-abc/session.jsonl.zstd' "$T/log" && ok "dsh: session file resolved" || bad "dsh: session file not resolved"
else echo "skip js template tests (node >= 22 not found)"; fi

[ "$fail" = 0 ] && echo "PASS" || { echo "FAILED"; exit 1; }
