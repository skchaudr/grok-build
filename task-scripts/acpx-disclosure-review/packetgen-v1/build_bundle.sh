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
  echo "## Facts the operator has not stated but the code shows"
  echo "The inspector has no Tab-to-pane focus. Tab already switches inspector tabs and stays that way. Today y copies the whole active tab (test copy_returns_active_tab_full_text); after this packet Y does that and y copies the cursor line or visual range."
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
