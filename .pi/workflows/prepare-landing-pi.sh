#!/bin/bash
# Open only packet preparation in a fresh, same-parent-model Pi. No builder launch.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
provider=${PI_PROVIDER:?Set PI_PROVIDER to the coordinating parent provider}
model=${PI_MODEL:?Set PI_MODEL to the coordinating parent model}
input=${1:-"$repo/.pi/workflows/landing-input.example.json"}
command -v rex >/dev/null
command -v pi >/dev/null
prompt="Start the Pi Workflow at $repo/.pi/workflows/prepare-landing.workflow.ts. Read its complete input from $input, set planner.provider to $provider and planner.model to $model (the actual parent model), preserve all other input values, then call workflow start once. End your turn after start and follow delivered contracts. This run only prepares detailed packets for F3, Cass, and Claude settings and waits at the protected human review gate. Do not launch builders, build, test the product, install, or modify source."
rex run -s "${REX_SESSION:?Run this from the intended Rex session}" -c "$repo" --label landing-packet-preparation --focus=false --shell=none -- pi --provider "$provider" --model "$model" --thinking high --approve "$prompt"
