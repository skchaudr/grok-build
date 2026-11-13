# Code Review slash commands (`/review-session`, `/review-message`)

**Status:** implemented (v1)  
**Date:** 2026-07-22  
**Scope:** native Grok Pager surfaces (not Pi RPC custom UI)

## Goal

Map PSM `psm-code-review` *mechanism* to grok-pi:

| Surface | Behavior |
|---|---|
| `/review-session` | Open two-pane review for whole session file changes |
| `/review-message` | Jump-style message picker; move = live jump scroll; Enter = review that turn |
| Default filter | `edit` + `write` only (`changes`) |
| Include reads | F2 `review_include_reads` (default off) + modal `r`; preview uses `for_read` |

## Architecture

- Data: scrollback `ToolCallBlock::Edit` (write = `Creating ` prefix)
- UI: native Pager modal (left file list, right full patch preview)
- Message pick: reuse `JumpState` + `JumpPurpose::Review`
- No adapter / Pi source changes; add names to `PI_GROK_NATIVE_COMMANDS`

## Non-goals (v1)

- Shell ops in UI (still reserved)
- Side-by-side split diff
- PSM settings (diffView, interceptExpand, …)

## Acceptance

- [x] `/review-session` lists unique changed paths; right pane shows full patch text
- [x] `/review-message` reuses jump list + live scroll; Enter opens turn-scoped review
- [x] Empty changes → toast, no empty modal
- [x] Unit test present in `views/review.rs` (lib tests blocked by pre-existing slash-command `billing_surface_visible` double-field errors)
- [x] `cargo check -p xai-grok-pager --lib` + `cargo check -p xai-grok-pager-bin --bin grok-pi` PASS

## Implementation notes

- Native Pager, not a Pi extension (RPC cannot host custom UI factories).
- `ReviewKindFilter::{Changes,All}`; `r` / F2 maps off→Changes, on→All. Shell still reserved.
- Allowlist: `PI_GROK_NATIVE_COMMANDS` includes `review-session` / `review-message`.
- **Tree mode (default OFF, persisted):** F2 `review_file_tree` + modal `t` toggles flat↔tree.
  Tree strips session `cwd` prefix and compacts single-child dir chains; consecutive
  Java package segments join with `.` (`com.example.app`).
- Right pane embeds `BlockViewerPane::for_edit` (same as Enter-on-edit): ListPane scroll / search `/` / filter `f` / wrap `w` / copy `y` / visual select / line-numbered unified diff.
- Default focus = **preview** so j/k/wheel scroll immediately; `n`/`p` switch files; `←`/Tab → file list; list `/` filters paths.
- Tree list scroll is sticky (`ensure_list_visible`): no force-center; when all rows fit, `list_view_start = 0` so the top is never clipped.
- Ctrl+click path (list row or preview title) opens the file via `link_opener::open_path`.
- Modal size ~98% overlay, top-biased, so it sits flush above the prompt/shortcuts row.
- File-type icons: auto when `nerd_fonts_available()` (same probe as branch icon / `GROK_NERD_FONTS`); no F2 toggle. Off → keep `+`/`~` tags only.
- **Include reads (default OFF, persisted):** F2 `review_include_reads` + modal `r`.
  Rebuilds file list from stored `entry_range`; read rows use tag `r` and `BlockViewerPane::for_read`.
  Same path with edit+read keeps edit as primary viewer and counts both ops.
