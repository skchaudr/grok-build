# Screen check: packet 3b and the error-line fix, installed build

Installed by `scripts/install-fork.sh` (grok-fork 3e36a47a8df9, commits `92f80438` and `d0eb35a3`). Opened `grok-fork`, `/acpx`, in its own Rex tab on the Mini, resized to 200x50 so the detail pane sits beside the stage tree. Run: `beta8-merge`, stage `cargo_check`.

- **Failure tail:** the Failure section starts `error: duplicate key`, `  --> crates/codegen/xai-grok-update/Cargo.toml:38:1`, `…`, then the tail from `38 | xai-tty-utils = …`.
- **Tab:** focus moves into the detail pane; the footer changes to `j/k line | v range | y copy | Y all | Esc tree | ? keys`.
- **Copy one line:** `j` ×4 to the error line, `y`: notice `Copied!`; `pbpaste` gives exactly `error: duplicate key`.
- **Copy a wrapped line from its second row:** `y` gives the whole line, `error: failed to load manifest for workspace member `/Volumes/…/beta8-rest/crates/codegen/xai-grok-pager``, with no added space or newline.
- **Range:** `v` plus `k` ×3, `y`: four lines, each once (`…`, `38 | …`, `   | ^^^…`, the wrapped error line whole).
- **Esc:** first ends visual mode (footer unchanged), second returns focus to the tree (run footer back, stage selection unchanged at 32/40), third leaves the run with `beta8-merge` still selected in the list.

Not checked: Shift+Tab, the narrow layout (Tab does nothing without a detail pane), paste on sab-air.
