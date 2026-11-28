# Screen check: installed build after packets 2, 3, 6

Installed by `scripts/install-fork.sh` (grok-fork 99e8cd736747), opened as `grok-fork` then `/acpx` in a Rex split on the Mini at about 80 columns. Keys used: arrows, Space, Enter (open run), Shift+M, Esc. No gate answered, no run started, no `y` (it would overwrite the clipboard).

## Seen working

- Flows groups finished runs by flow: `▸ mixed (8)`, `▸ mixed-talk (11)`, `▸ mixed-demo (14)`, `▸ land: slugify (8)`, `▸ monitor (2)`; 75 runs show as 36 rows.
- Space on `mixed (8)` opens it (`▾`), its 7 older runs appear indented, rows go 36 → 43; Space again folds it.
- At this width the selected row shows its executed path on the line below, e.g. `✓ implement → ✓ test → ✓ review → ✓ accept`, and a waiting run's question wraps under the row.
- `beta8-merge`, Enter, select `cargo_check`, Shift+M: detail starts with `── Failure ──` (the saved Cargo error), then `Enter inspect`, then `── Command ──`. Header: `✗ cargo_check 115ms/1h30m`, `action · shell · attempt 1`.
- Esc from the run returns to the same `beta8-merge` row (11/36).

## Problems seen

1. **Failure tail drops the root-cause line.** `beta8-merge`'s saved output starts `error: duplicate key` / `--> crates/codegen/xai-grok-update/Cargo.toml:38:1`; the digest starts at `38 | xai-tty-utils = { workspace = true }`, so the first and most useful error line is cut. Cause: `step_digest::trim_failure_tail` keeps the last `DEFAULT_FAILURE_MAX_LINES` non-empty lines. Suggested fix: when the tail is cut, keep the first `error` line and its `-->` location ahead of the kept tail.
2. **Ledger row still reads `Workflow run cancell…`** for `beta8-merge` instead of the `cargo_check` failure. No packet owns this yet.
3. **Needs You still lists all 10 waits individually.** Expected: this return used the ledger spec from before `8f7c5ca8`; Sol's re-rendered packet 2 groups waits.

## Not seen

- Full width (side preview, `N steps ran` in the row): the pane was about 80 columns, below the count's width threshold.
- Packet 6 pickers (bare `/open`, `/flow`) in ACPX chat.
- `y` copy and paste.
