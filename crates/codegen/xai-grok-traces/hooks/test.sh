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
expect 15 "$(cnt "$S" 'grok-traces" hook claude')" "claude: one entry per event after two applies"
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

G() { command -v rg >/dev/null && rg "$@" || command grep -E "$@"; }
# ---- cursor / dsh / bb, in a second synthetic HOME ----
REAL_HOME="$HOME"; export HOME="$T/home2"; mkdir -p "$HOME/.cursor" "$HOME/.grok/bin" "$HOME/.dsh/profiles/web" "$HOME/.dsh/profiles/tui" "$HOME/.dsh/plugins" "$HOME/.bb" "$T/stub"
printf '#!/bin/sh\nexit 0\n' > "$HOME/.grok/bin/grok-traces"; chmod +x "$HOME/.grok/bin/grok-traces"
cat > "$HOME/.cursor/hooks.json" <<'J'
{"version": 1, "hooks": {
  "afterAgentResponse": [{"command": "'/opt/homebrew/bin/moshi-hook' cursor-hook"}],
  "beforeSubmitPrompt": [{"command": "orca-cursor", "timeout": 10}],
  "stop": [{"command": "orca-cursor", "timeout": 10}]
}}
J
H="$HOME/.cursor/hooks.json"; cp "$H" "$T/H.orig"
printf -- '- insert:\n    # note\n    - id: moshi-bridge\n      name: "./m.mjs"\n- id: x\n  config: {a: 1}\n' > "$HOME/.dsh/profiles/web/cordis.patch.yml"
printf -- '# only comments\n- id: llm\n  config:\n    k: v' > "$HOME/.dsh/profiles/tui/cordis.patch.yml"   # no trailing newline, no insert
cp "$HOME/.dsh/profiles/web/cordis.patch.yml" "$T/web.orig"; cp "$HOME/.dsh/profiles/tui/cordis.patch.yml" "$T/tui.orig"
cat > "$T/stub/bb" <<'SH'
#!/bin/sh
echo "bb $*" >> "$BB_LOG"
if [ "$1 $2" = "plugin list" ]; then echo '{"plugins":[]}'; fi
SH
chmod +x "$T/stub/bb"; export BB_LOG="$T/bb.log"; : > "$BB_LOG"; OLDPATH="$PATH"; export PATH="$T/stub:$PATH"

out="$("$HERE/install.sh" --dsh-profile web)"
cmp -s "$H" "$T/H.orig" && cmp -s "$HOME/.dsh/profiles/web/cordis.patch.yml" "$T/web.orig" && [ ! -e "$HOME/.dsh/plugins/grok-traces.mjs" ] && [ ! -s "$BB_LOG" ] && ok "cursor/dsh/bb dry-run writes nothing" || bad "dry-run wrote (cursor/dsh/bb)"
echo "$out" | G -q 'would run: bb plugin install' && ok "bb dry-run prints install command" || bad "bb dry-run output"
"$HERE/install.sh" --apply --dsh-profile web --dsh-profile tui >/dev/null
"$HERE/install.sh" --apply --dsh-profile web --dsh-profile tui >/dev/null   # idempotent
expect 18 "$(cnt "$H" 'grok-traces" hook cursor')" "cursor: 18 events wired, once after two applies"
expect "afterAgentResponse,afterAgentThought,afterFileEdit,afterMCPExecution,afterShellExecution,beforeMCPExecution,beforeReadFile,beforeShellExecution,beforeSubmitPrompt,postToolUse,postToolUseFailure,preCompact,preToolUse,sessionEnd,sessionStart,stop,subagentStart,subagentStop" "$(jq -r '[.hooks|to_entries[]|select(.value|any(.command|contains("grok-traces")))|.key]|sort|join(",")' "$H")" "cursor: event set"
expect 1 "$(jq '[.hooks.beforeSubmitPrompt[]|select(.command|contains("grok-traces"))]|length' "$H")" "cursor: gating beforeSubmitPrompt wired"
expect 1 "$(jq '[.hooks.beforeSubmitPrompt[]|select(.command=="orca-cursor")]|length' "$H")" "cursor: orca beforeSubmitPrompt hook survives"
expect 1 "$(cnt "$H" moshi-hook)" "cursor: moshi untouched"
expect 2 "$(cnt "$H" orca-cursor)" "cursor: orca untouched"
expect 1 "$(jq .version "$H")" "cursor: version preserved"
W="$HOME/.dsh/profiles/web/cordis.patch.yml"; U="$HOME/.dsh/profiles/tui/cordis.patch.yml"
expect 1 "$(G -c 'grok-traces begin' "$W")" "dsh web: one block after two applies"
[ "$(sed -n 2p "$W")" = "    # grok-traces begin" ] && G -q "name: '$HOME/.dsh/plugins/grok-traces.mjs'" "$W" && ok "dsh web: block injected into existing insert list" || bad "dsh web injection"
G -q '^- insert:$' "$U" && tail -1 "$U" | G -q 'grok-traces end' && ok "dsh tui: new insert block appended" || bad "dsh tui append"
cmp -s "$HERE/dsh/traces.mjs" "$HOME/.dsh/plugins/grok-traces.mjs" && ok "dsh: plugin copied" || bad "dsh plugin copy"
ls "$HOME"/.dsh/profiles/web/cordis.patch.yml.bak-traces-* >/dev/null 2>&1 && ok "dsh: backup written" || bad "dsh backup"
G -q "bb plugin install $HOME/.grok/hooks/bb --yes" "$BB_LOG" && cmp -s "$HERE/bb/server.ts" "$HOME/.grok/hooks/bb/server.ts" && ok "bb: files copied, plugin install run" || bad "bb apply"
out="$("$HERE/install.sh" --check)"; echo "$out" | G -q 'cursor : .*stop' && echo "$out" | G -q 'dsh    : profiles: .*tui' && ok "check reports cursor + dsh" || bad "check cursor/dsh: $out"
"$HERE/install.sh" --remove >/dev/null
[ "$(jq -S . "$H")" = "$(jq -S . "$T/H.orig")" ] && ok "cursor: --remove restores content" || bad "cursor remove"
cmp -s "$W" "$T/web.orig" && ok "dsh web: --remove restores byte-identical" || bad "dsh web remove"
[ "$(cat "$U")" = "$(cat "$T/tui.orig")" ] && ok "dsh tui: --remove restores content" || bad "dsh tui remove"
[ ! -e "$HOME/.dsh/plugins/grok-traces.mjs" ] && ok "dsh: plugin file removed with last profile" || bad "dsh plugin remains"
[ ! -e "$HOME/.grok/hooks/bb/server.ts" ] && ok "bb: files removed" || bad "bb files remain"
# a bb CLI that is a client of another host must not be touched
: > "$BB_LOG"; echo '{"serverUrl":"http://100.1.2.3:38886"}' > "$HOME/.bb/config.json"
out="$("$HERE/install.sh" --apply)"; echo "$out" | G -q '\[bb\] skipped: .*100.1.2.3' && [ ! -s "$BB_LOG" ] && ok "bb: remote-client CLI skipped" || bad "bb remote skip"
export PATH="$OLDPATH"; export HOME="$REAL_HOME"

# JS templates: spawn args, stdin payload, debounce (needs node >= 22 for type stripping; skipped otherwise)
if command -v node >/dev/null && node --experimental-strip-types -e 0 2>/dev/null; then
  printf '#!/bin/sh\necho "$* $(cat)" >> "%s/log"\n' "$T" > "$HOME/.grok/bin/grok-traces"
  mkdir -p "$HOME/.dsh/sessions/--p--/session-abc"; : > "$HOME/.dsh/sessions/--p--/session-abc/session.jsonl.zstd"
  cat > "$T/run.mjs" <<'JS'
const [kind, file] = process.argv.slice(2); const mod = (await import(file)).default; const h = {}
const wait = ms => new Promise(r => setTimeout(r, ms))
if (kind === 'bb') {
  await mod({ events: { on: (e, f) => { h[e] = f } }, onDispose() {} })
  for (let i = 0; i < 5; i++) h['thread.idle']({ thread: { id: 'thr_1' } })
} else if (kind === 'dsh') {
  mod.apply({ on: (e, f) => { h[e] = f } }); const s = { id: 'session-abc', header: { cwd: '/w' } }
  for (let i = 0; i < 5; i++) h['session/event'](s, { type: 'tool/call', data: { name: 'bash' } })
} else {
  mod({ on: (e, f) => { h[e] = f } }); const c = { cwd: '/w', sessionManager: { getSessionId: () => 'S', getSessionFile: () => '/f' } }
  h.session_start({}, c); await wait(600)
  for (let i = 0; i < 5; i++) { h.tool_result({ toolName: 'bash' }, c); await wait(20) }
}
await wait(1500)
JS
  for k in omp pi bb dsh; do
    f="$HERE/$k/traces.ts"; [ "$k" = dsh ] && f="$HERE/dsh/traces.mjs"; [ "$k" = bb ] && f="$HERE/bb/server.ts"
    : > "$T/log"; node --experimental-strip-types --no-warnings "$T/run.mjs" "$k" "$f"
    grep -q "^hook $k " "$T/log" && ok "$k: spawns 'hook $k'" || bad "$k: no spawn"
    n="$(wc -l < "$T/log" | tr -d ' ')"
    case $k in dsh|bb) want=2 ;; *) want=3 ;; esac   # leading + one trailing (pi/omp add session_start)
    expect $want "$n" "$k: burst debounced to leading + trailing"
  done
  grep -q 'session-abc/session.jsonl.zstd' "$T/log" && ok "dsh: session file resolved" || bad "dsh: session file not resolved"
  : > "$T/log"; node --experimental-strip-types --no-warnings "$T/run.mjs" bb "$HERE/bb/server.ts"
  G -q '^hook bb \{"event":"thread.idle","thread_id":"thr_1"\}' "$T/log" && ok "bb: payload carries event + thread_id" || bad "bb payload"
else echo "skip js template tests (node >= 22 not found)"; fi

[ "$fail" = 0 ] && echo "PASS" || { echo "FAILED"; exit 1; }
