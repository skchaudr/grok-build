#!/usr/bin/env bash
# Eval for the 2026-10 upstream sync of the pager fork.
# Fails until upstream/main is an ancestor of HEAD and the three external-agent
# auth tests from b6a12331 are still present.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

file=crates/codegen/xai-grok-pager/src/acp/mod.rs
fail=0

if git merge-base --is-ancestor upstream/main HEAD; then
  echo "OK: upstream/main ($(git rev-parse --short upstream/main)) is an ancestor of HEAD"
else
  echo "FAIL: upstream/main ($(git rev-parse --short upstream/main)) is not an ancestor of HEAD ($(git rev-parse --short HEAD))"
  fail=1
fi

tests=(
  external_agent_empty_auth_methods_is_ready_without_authentication
  builtin_agent_empty_auth_methods_still_requires_login
  external_agent_advertised_login_is_preserved
)

for name in "${tests[@]}"; do
  if grep -q "fn ${name}(" "$file"; then
    echo "OK: recorded test ${name}"
  else
    echo "FAIL: missing test ${name}"
    fail=1
  fi
done

if grep -q 'long = "agent-cmd"' crates/codegen/xai-grok-pager/src/app/cli.rs; then
  echo "OK: --agent-cmd flag present"
else
  echo "FAIL: --agent-cmd flag missing from cli.rs"
  fail=1
fi

exit "$fail"
