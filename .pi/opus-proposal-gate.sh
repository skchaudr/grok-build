#!/bin/sh
# Typed gate: runs the verify command given as arguments, always exits 0, prints one JSON line {ok, output, diff}.
# Kept under pi-subagents' 12000-char typed-gate cap: output <= 2000, diff <= 5000 (larger diffs are rejected).
cd "$(dirname "$0")/.." || exit 0
out=$( eval "$*" 2>&1 ); st=$?
# intent-to-add so files the worker created show in the diff
git add -N -- . ':!.pi' ':!.grok-pi' 2>/dev/null
diff=$(git diff -- . ':!.pi' ':!.grok-pi')
node -e '
const [st, out, diff] = process.argv.slice(1);
let ok = st === "0", output = out.slice(-2000), d = diff;
if (!diff) { ok = false; output = "no diff"; }
else if (diff.length > 5000) { ok = false; output = `step too big (${diff.length} chars): split the brief`; d = ""; }
console.log(JSON.stringify({ ok, output, diff: d }));
' "$st" "$out" "$diff"
