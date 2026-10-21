#!/usr/bin/env bash
# Wire grok-traces into the harnesses present on this machine. Dry-run by default.
#   install.sh [--dry-run] | --apply | --remove | --check   [--retire-pi-observability] [--even-if-missing] [--dsh-profile NAME]...
# Every JSON file is backed up to <file>.bak-traces-<ts>, edited via jq into a temp file, validated, then moved in.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HOME/.grok/bin/grok-traces"
TS="$(date +%Y%m%d-%H%M%S)"
MODE=dry; RETIRE=0; EVEN=0; DSH_PROFILES=(); NEXT_PROFILE=0
for a in "$@"; do
  if [ "$NEXT_PROFILE" = 1 ]; then DSH_PROFILES+=("$a"); NEXT_PROFILE=0; continue; fi
  case "$a" in
    --dsh-profile) NEXT_PROFILE=1 ;; --dsh-profile=*) DSH_PROFILES+=("${a#*=}") ;;
    --dry-run) MODE=dry ;; --apply) MODE=apply ;; --remove) MODE=remove ;; --check) MODE=check ;;
    --retire-pi-observability) RETIRE=1 ;; --even-if-missing) EVEN=1 ;;
    -h|--help) sed -n 2,4p "$0"; exit 0 ;;
    *) echo "unknown flag: $a" >&2; exit 2 ;;
  esac
done
[ "$NEXT_PROFILE" = 1 ] && { echo "--dsh-profile needs a name" >&2; exit 2; }
for p in ${DSH_PROFILES[@]+"${DSH_PROFILES[@]}"}; do
  case "$p" in ""|*/*|.*) echo "bad --dsh-profile: $p" >&2; exit 2 ;; esac
done
command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }
WRITE=0; [ "$MODE" = apply ] && WRITE=1
[ "$MODE" = remove ] && WRITE=1
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

# name matcher? (matcher "*" only where the event takes one)
CLAUDE_EVENTS='[["SessionStart",1],["UserPromptSubmit",0],["PreToolUse",1],["PostToolUse",1],["PostToolUseFailure",1],["Stop",0],["SubagentStop",1],["SessionEnd",1],["PermissionRequest",1],["PermissionDenied",1],["Notification",1],["SubagentStart",1],["PreCompact",1],["PostCompact",1],["StopFailure",1]]'
# Cursor hooks.json is flat ({"hooks":{"<event>":[{"command":..}]}}). Non-gating events only: `{}` is a no-op reply for
# every one of them. beforeSubmitPrompt/preToolUse/before* gate and need {"continue":true}/{"permission":"allow"}: not wired.
CURSOR_EVENTS='[["afterAgentResponse",0],["postToolUse",0],["postToolUseFailure",0],["stop",0]]'
CODEX_EVENTS='[["SessionStart",1],["UserPromptSubmit",0],["PreToolUse",1],["PostToolUse",1],["PostToolUseFailure",1],["Stop",0],["SubagentStop",1]]'

JQ_DEFS='
def ours: contains("grok-traces\" hook ");
def stale: contains("log-commands.sh");
def strip(f):
  if .hooks == null then . else
    .hooks |= (with_entries(
      .key as $k | .value as $groups |
      ($groups | map(
        if (.hooks | type) == "array" then
          . as $g | ($g.hooks | map(select(((.command? // "") | f) | not))) as $kept |
          if ($kept | length) == 0 and ($g.hooks | length) > 0 then empty else ($g | .hooks = $kept) end
        else . end)) as $out |
      if ($out | length) == 0 and ($groups | length) > 0 then empty else (.value = $out) end))
  end;
def add_ours($cmd; $async; $events):
  .hooks //= {} |
  reduce $events[] as $e (.;
    .hooks[$e[0]] = ((.hooks[$e[0]] // []) + [
      ({} | (if $e[1] == 1 then .matcher = "*" else . end)
          | .hooks = [{type: "command", command: $cmd} + (if $async then {async: true} else {} end)])]));
def cstrip(f):
  if .hooks == null then . else
    .hooks |= (with_entries(.value |= map(select(((.command? // "") | f) | not))) | with_entries(select(.value | length > 0)))
  end;
def cadd($cmd; $events):
  .hooks //= {} |
  reduce $events[] as $e (.; .hooks[$e[0]] = ((.hooks[$e[0]] // []) + [{command: $cmd, timeout: 10}]));
'

resolve() { local p="$1"; if [ -L "$p" ]; then readlink -f "$p" 2>/dev/null || echo "$p"; else echo "$p"; fi; }
note() { printf '%s\n' "$*"; }

# jq_file <label> <file> <source> <events-json> <async 0|1> [flat]
jq_file() {
  local label="$1" f="$2" src="$3" events="$4" async="$5" flat="${6:-}" cmd new
  [ -f "$f" ] || { note "[$label] not present ($f), skipped"; return 0; }
  jq empty "$f" || { note "[$label] $f is not valid JSON, skipped"; return 0; }
  cmd="\"\$HOME/.grok/bin/grok-traces\" hook $src"
  new="$WORK/$label.new"
  if [ "$flat" = flat ]; then
    if [ "$MODE" = remove ]; then jq "$JQ_DEFS"'cstrip(ours)' "$f" > "$new"
    else jq --arg cmd "$cmd" --argjson ev "$events" "$JQ_DEFS"'cstrip(stale) | cstrip(ours) | cadd($cmd; $ev)' "$f" > "$new"; fi
  elif [ "$MODE" = remove ]; then
    jq "$JQ_DEFS"'strip(ours)' "$f" > "$new"
  else
    jq --arg cmd "$cmd" --argjson ev "$events" --argjson async "$async" \
      "$JQ_DEFS"'strip(stale) | strip(ours) | add_ours($cmd; ($async == 1); $ev)' "$f" > "$new"
  fi
  jq empty "$new" || { note "[$label] generated JSON invalid, aborting"; exit 1; }
  if cmp -s "$f" "$new"; then note "[$label] $f: unchanged"; return 0; fi
  if [ "$WRITE" = 1 ]; then
    local real; real="$(resolve "$f")"
    cp -p "$real" "$real.bak-traces-$TS"
    cp -p "$real" "$real.traces-tmp.$$" && cat "$new" > "$real.traces-tmp.$$" && mv "$real.traces-tmp.$$" "$real"
    note "[$label] $f: updated (backup $real.bak-traces-$TS)"
  else
    note "[$label] $f: would change"
    diff -u --label "$f" --label "$f (new)" "$f" "$new" || true
  fi
}

# copy_file <label> <template> <dest>
copy_file() {
  local label="$1" src="$2" dest="$3"
  if [ "$MODE" = remove ]; then
    if [ ! -e "$dest" ]; then note "[$label] $dest: absent"
    elif cmp -s "$src" "$dest"; then rm -f "$dest"; note "[$label] $dest: removed"
    else note "[$label] $dest: differs from template, left in place"; fi
    return 0
  fi
  if [ -f "$dest" ] && cmp -s "$src" "$dest"; then note "[$label] $dest: unchanged"; return 0; fi
  if [ "$WRITE" = 1 ]; then
    mkdir -p "$(dirname "$dest")"
    [ -f "$dest" ] && cp -p "$dest" "$dest.bak-traces-$TS"
    cp "$src" "$dest"; note "[$label] $dest: installed"
  else
    note "[$label] $dest: would install"
    local cur="$dest"; [ -f "$dest" ] || cur=/dev/null
    diff -u --label "$dest" --label "$src" "$cur" "$src" || true
  fi
}

# pi-observability symlink in an OMP extensions dir
retire_obs() {
  local dir="$1" link="$1/pi-observability.ts"
  if [ "$MODE" = remove ]; then
    [ "$RETIRE" = 1 ] || return 0
    local bak; bak="$(ls -d "$link".retired-traces-* 2>/dev/null | tail -1 || true)"
    if [ -n "$bak" ] && [ ! -e "$link" ] && [ ! -L "$link" ]; then mv "$bak" "$link"; note "[omp] restored $link"; fi
    return 0
  fi
  [ -L "$link" ] || [ -e "$link" ] || return 0
  if [ "$RETIRE" != 1 ]; then note "[omp] $link present (kept; pass --apply --retire-pi-observability to retire it)"; return 0; fi
  if [ "$WRITE" = 1 ]; then mv "$link" "$link.retired-traces-$TS"; note "[omp] retired $link -> $link.retired-traces-$TS"
  else note "[omp] would retire $link -> $link.retired-traces-$TS"; fi
}

installed_in() { # does <json> contain our hook; prints event list
  [ -f "$1" ] && jq -r "$JQ_DEFS"'(.hooks // {}) | to_entries | map(select(.value | any(.[]?.hooks[]?; (.command? // "") | ours)) | .key) | join(",")' "$1" 2>/dev/null || true
}

# ---- bb: plugin installed through bb's own mechanism, only where this host runs the bb server ----
BB_PLUGIN_ID="grok-traces-hook"; BB_DEST="$HOME/.grok/hooks/bb"; BB_WHY=""
bb_local() {
  command -v bb >/dev/null && [ -d "$HOME/.bb" ] || { BB_WHY="no bb or ~/.bb"; return 1; }
  local url; url="$(jq -r '.serverUrl // empty' "$HOME/.bb/config.json" 2>/dev/null || true)"
  [ -z "$url" ] || { BB_WHY="bb CLI here is a client of $url; run install.sh on that host"; return 1; }
}
bb_installed() { bb plugin list --json 2>/dev/null | jq -e --arg id "$BB_PLUGIN_ID" '.plugins | any(.[]; .id == $id)' >/dev/null 2>&1; }
bb_files() { copy_file bb-files "$HERE/bb/server.ts" "$BB_DEST/server.ts"; copy_file bb-files "$HERE/bb/package.json" "$BB_DEST/package.json"; }
bb_plugin() {
  bb_local || { note "[bb] skipped: $BB_WHY"; return 0; }
  if [ "$MODE" = remove ]; then
    if bb_installed; then bb plugin remove "$BB_PLUGIN_ID" || note "[bb] remove failed"; else note "[bb] $BB_PLUGIN_ID: not installed"; fi
    bb_files; return 0
  fi
  bb_files
  if [ "$WRITE" = 1 ]; then
    if bb_installed; then bb plugin reload "$BB_PLUGIN_ID" >/dev/null 2>&1 || true; note "[bb] $BB_PLUGIN_ID: already installed, reloaded"
    elif bb plugin install "$BB_DEST" --yes; then bb plugin reload "$BB_PLUGIN_ID" >/dev/null 2>&1 || true; else note "[bb] install failed (files left in $BB_DEST)"; fi
  else note "[bb] would run: bb plugin install $BB_DEST --yes   (then bb plugin reload $BB_PLUGIN_ID)"; fi
}

# ---- dsh: mount ~/.dsh/plugins/grok-traces.mjs from a profile's cordis.patch.yml, between marker comments ----
DSH_PLUGIN="$HOME/.dsh/plugins/grok-traces.mjs"
dsh_wired_profiles() {
  local f out=""
  for f in "$HOME"/.dsh/profiles/*/cordis.patch.yml; do
    if [ -f "$f" ] && grep -q '^ *# grok-traces begin' "$f"; then out="$out${out:+,}$(basename "$(dirname "$f")")"; fi
  done
  printf '%s' "$out"
}
# dsh_patch <in> <out> add|remove: drops any marked block; "add" re-inserts it after an existing top-level `- insert:` or at EOF
dsh_patch() {
  awk -v mode="$3" -v plugin="$DSH_PLUGIN" '
    /^ *# grok-traces begin/ { skip = 1; next }
    /^ *# grok-traces end/   { skip = 0; next }
    skip { next }
    { lines[++n] = $0 }
    END {
      at = 0; ind = "    "
      if (mode == "add") {
        for (i = 1; i <= n; i++) if (lines[i] ~ /^- insert:[ \t]*$/) { at = i; break }
        if (at) for (j = at + 1; j <= n; j++) {
          if (lines[j] ~ /^[ \t]+- id:/) { match(lines[j], /^[ \t]+/); ind = substr(lines[j], 1, RLENGTH); break }
          if (lines[j] ~ /^[^ \t#]/) break
        }
      }
      for (i = 1; i <= n; i++) {
        print lines[i]
        if (i == at) printf "%s# grok-traces begin\n%s- id: grok-traces\n%s  name: \047%s\047\n%s# grok-traces end\n", ind, ind, ind, plugin, ind
      }
      if (mode == "add" && !at) printf "# grok-traces begin\n- insert:\n    - id: grok-traces\n      name: \047%s\047\n# grok-traces end\n", plugin
    }' "$1" > "$2"
}
dsh_profile() {
  local name="$1" f="$HOME/.dsh/profiles/$1/cordis.patch.yml" new="$WORK/dsh-$1.new"
  [ -f "$f" ] || { note "[dsh] profile '$name': $f not found, skipped"; return 0; }
  if [ "$(grep -c '^ *# grok-traces begin' "$f")" -gt "$(grep -c '^ *# grok-traces end' "$f")" ]; then
    note "!! [dsh] profile '$name': $f has '# grok-traces begin' with no matching end. REFUSED, file left untouched. Fix the markers by hand."; return 0
  fi
  if [ "$MODE" != remove ] && ! grep -q '^ *# grok-traces begin' "$f" && grep -v '^ *#' "$f" | grep -q 'id: grok-traces'; then
    note "[dsh] profile '$name': an unmanaged 'grok-traces' entry exists, skipped"; return 0
  fi
  if [ "$MODE" = remove ]; then dsh_patch "$f" "$new" remove; else dsh_patch "$f" "$new" add; fi
  if cmp -s "$f" "$new"; then note "[dsh] $f: unchanged"; return 0; fi
  if [ "$WRITE" = 1 ]; then
    cp -p "$f" "$f.bak-traces-$TS" && cat "$new" > "$f.traces-tmp.$$" && mv "$f.traces-tmp.$$" "$f"
    note "[dsh] $f: updated (backup $f.bak-traces-$TS)"
  else
    note "[dsh] $f: would change"
    diff -u --label "$f" --label "$f (new)" "$f" "$new" || true
  fi
}
dsh_wire() {
  [ -d "$HOME/.dsh/profiles" ] || { note "[dsh] not present, skipped"; return 0; }
  local profs=(${DSH_PROFILES[@]+"${DSH_PROFILES[@]}"}) p have
  if [ "$MODE" = remove ] && [ ${#profs[@]} = 0 ]; then
    have="$(dsh_wired_profiles)"; [ -z "$have" ] || IFS=, read -ra profs <<<"$have"
  fi
  if [ ${#profs[@]} = 0 ]; then
    have=""; for p in "$HOME"/.dsh/profiles/*/cordis.patch.yml; do [ -f "$p" ] && have="$have$(basename "$(dirname "$p")") "; done
    note "[dsh] not wired: pass --dsh-profile NAME (profiles with a patch file: ${have:-none})"
    [ "$MODE" = remove ] && copy_file dsh-plugin "$HERE/dsh/traces.mjs" "$DSH_PLUGIN"
    return 0
  fi
  [ "$MODE" = remove ] || copy_file dsh-plugin "$HERE/dsh/traces.mjs" "$DSH_PLUGIN"
  for p in "${profs[@]}"; do dsh_profile "$p"; done
  if [ "$MODE" = remove ] && [ -z "$(dsh_wired_profiles)" ]; then copy_file dsh-plugin "$HERE/dsh/traces.mjs" "$DSH_PLUGIN"; fi
  return 0
}

check() {
  local ev
  if [ -x "$BIN" ]; then note "binary: $BIN (present)"; else note "binary: $BIN MISSING"; fi
  ev="$(installed_in "$HOME/.claude/settings.json")"; note "claude : ${ev:-not wired}"
  ev="$(installed_in "$HOME/.codex/hooks.json")";     note "codex  : ${ev:-not wired}"
  local f
  f="$HOME/.grok/hooks/traces.json"; if [ -f "$f" ]; then note "grok   : wired ($f)"; else note "grok   : not wired"; fi
  f="$HOME/.omp/agent/extensions/traces.ts"; if [ -f "$f" ]; then note "omp    : wired"; else note "omp    : not wired"; fi
  f="$HOME/.pi/agent/extensions/traces.ts"; if [ -f "$f" ]; then note "pi     : wired"; else note "pi     : not wired"; fi
  ev="$(jq -r "$JQ_DEFS"'(.hooks // {}) | to_entries | map(select(.value | any(.[]?; (.command? // "") | ours)) | .key) | join(",")' "$HOME/.cursor/hooks.json" 2>/dev/null || true)"; note "cursor : ${ev:-not wired}"
  local prof; prof="$(dsh_wired_profiles)"; if [ -n "$prof" ]; then note "dsh    : profiles: $prof"; else note "dsh    : not wired (use --dsh-profile NAME)"; fi
  if bb_local; then note "bb     : $(bb_installed && echo wired || echo not wired)"; else note "bb     : n/a ($BB_WHY)"; fi
  if [ -x "$BIN" ]; then "$BIN" doctor || true; fi
}

if [ "$MODE" = check ]; then check; exit 0; fi

if [ "$MODE" = apply ] && [ ! -x "$BIN" ] && [ "$EVEN" != 1 ]; then
  note "refusing --apply: $BIN is not installed (hooks would error on every event). Install it first or pass --even-if-missing." >&2; exit 1
fi
[ -x "$BIN" ] || note "warning: $BIN not installed yet"
note "mode: $MODE"

jq_file claude "$HOME/.claude/settings.json" claude "$CLAUDE_EVENTS" 1
jq_file codex  "$HOME/.codex/hooks.json"     codex  "$CODEX_EVENTS"  0
[ -d "$HOME/.grok" ] && copy_file grok "$HERE/grok/traces.json" "$HOME/.grok/hooks/traces.json" || note "[grok] not present, skipped"
if [ -d "$HOME/.omp/agent" ]; then
  copy_file omp "$HERE/omp/traces.ts" "$HOME/.omp/agent/extensions/traces.ts"
  retire_obs "$HOME/.omp/agent/extensions"
else note "[omp] not present, skipped"; fi
if [ -d "$HOME/.pi/agent" ]; then copy_file pi "$HERE/pi/traces.ts" "$HOME/.pi/agent/extensions/traces.ts"; else note "[pi] not present, skipped"; fi
jq_file cursor "$HOME/.cursor/hooks.json" cursor "$CURSOR_EVENTS" 0 flat
dsh_wire
bb_plugin
[ "$MODE" = dry ] && note "dry-run only; pass --apply to write"
exit 0
