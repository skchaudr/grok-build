#!/bin/sh
# Builds the two generated context files for the packet-writer run.
# Run from the repository root. Output is committed beside this script.
set -eu
OUT=task-scripts/acpx-disclosure-review/packetgen-v1/_bundle
PK=task-scripts/acpx-disclosure-packets.md
VIEWS=crates/codegen/xai-grok-pager/src/views
HEAD_SHA=$(git rev-parse --short HEAD)
ITEMS='^\s*(pub(\(crate\))? )?(fn|struct|enum|impl|const|static|mod) |^\s*#\[cfg\(test\)\]'

{
  echo "# Objective: packet 7, inspector body cursor and range"
  echo
  echo "You are writing the task packet (not the implementation) for the packet below. Facts as of $HEAD_SHA on main. Packet 3b (detail-pane focus and linewise copy) has landed in 92f80438, was used on the installed build, and copied exact lines, wrapped lines and ranges. Its implementation is in views/acpx_modal/detail.rs and is the pattern packet 7 reuses."
  echo
  echo "## The packet as the operator wrote it"
  sed -n '74,76p' "$PK"
  echo
  echo "## The shared key contract (verbatim, from the focus and copy track)"
  sed -n '60,66p' "$PK"
  echo
  echo "## Coordinator integration item (verbatim)"
  sed -n '86p' "$PK"
  echo
  echo "## Defects the operator hit on the installed build (sab-air, 2026-11-27 23:29), in his words"
  echo "He opened /acpx, a past run, one step with Enter (the inspector), and copied output. The page looks good and works badly:"
  echo "1. To read another step he has to press Esc, go down a step and press Enter again; he wants to open one step and use the arrow keys to reach the others."
  echo "2. Tab on a step moves between the step's output and its meta (inspector tabs). That is expected; it stays."
  echo "3. y copies the whole tab, one line or fifty. He wants the same linewise copy the grok build line viewer has."
  echo
  echo "## What the code shows about those defects"
  echo "Step switching already exists as [ and ] (STEP_KEYS: Previous / next step in run), listed only in the ? overlay, and Left/Right are bound to horizontal scroll (also h/l). Up/Down and j/k scroll today and become the line cursor under the contract, so they cannot also change steps. Today y copies the whole active tab (test copy_returns_active_tab_full_text); after this packet Y does that and y copies the cursor line or visual range."
  echo
  echo "## Decision made for the writer (the coordinator's pick; the operator may overrule)"
  echo "Left and Right switch to the previous and next step, like [ and ]; h and l keep horizontal scroll. The footer of the inspector shows the step keys and the cursor keys while it is open."
} > "$OUT/objective.md"

{
  echo "# Repository map (line numbers at HEAD $HEAD_SHA)"
  echo
  echo "File sizes in lines:"
  wc -l "$VIEWS/acpx_modal.rs" "$VIEWS/acpx_modal/inspector.rs" "$VIEWS/acpx_modal/detail.rs" | sed 's#.*/views/#  #'
  for f in acpx_modal/inspector.rs acpx_modal.rs acpx_modal/detail.rs; do
    echo
    echo "## views/$f: items (line: text)"
    rg -n "$ITEMS" "$VIEWS/$f" | sed 's/^/  /'
  done
} > "$OUT/repo-map.md"
wc -c "$OUT/objective.md" "$OUT/repo-map.md"
