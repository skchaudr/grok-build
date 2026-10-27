#!/bin/sh
# Lane gate: sh .pi/gate.sh "<verify command>". Runs the command, exits 0 always, prints one
# JSON line {ok, output}; output is the last 2000 chars of stdout+stderr. The caller owns the timeout.
if [ $# -eq 0 ]; then echo '{"ok":false,"output":"no verify command"}'; exit 0; fi
tmp=$(mktemp)
eval "$*" >"$tmp" 2>&1; st=$?
node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.stringify({ok:process.argv[1]==="0",output:s.slice(-2000)})))' "$st" <"$tmp"
rm -f "$tmp"
exit 0
