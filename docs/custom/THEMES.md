# Zed theme ports

Four Zed themes ported faithfully: `kanso-mist` (Kanso Mist, Borderless + No italics), `kanagawa-wave`, `kanagawa`, `jetbrains-islands-dark`. Select with `/theme kanso-mist` or `[ui] theme = "kanso-mist"`. All need truecolor.

Generated, not hand-written: `scripts/themes/zed_port.py <zed-themes-dir> <repo-root>` writes `crates/codegen/xai-grok-pager-render/src/theme/zed_ports.rs` and `assets/<slug>.tmTheme`. Source JSON is not vendored; copy it read-only from `~/Library/Application Support/Zed/extensions/installed/{kanso,kanagawa-themes,jetbrains-themes}/themes/`. Translucent Zed colors are composited over `editor.background`; alpha-0 colors are skipped in fallback chains.

## UI mapping (Grok field ← Zed key, first visible wins)

| Grok | Zed |
|---|---|
| bg_base | editor.background |
| bg_light (user block) | editor.active_line.background → element.hover (first that differs from bg) |
| bg_dark, md_code_bg, paste_bg | elevated_surface → surface → panel → active_line → element.hover (first that differs from bg) |
| bg_highlight / bg_hover | element.selected / element.hover |
| bg_visual | players[0].selection |
| text_primary / text_secondary, md_text, accent_user | text / editor.foreground |
| gray_dim / gray / gray_bright | text.placeholder / syntax.comment / text.muted |
| accent_assistant, accent_thinking, accent_running | syntax.keyword |
| accent_system, accent_skill / error / success / warning | info / error / success / warning |
| accent_tool, paste_fg, md_task_unchecked | text.muted |
| command / running / accent_verify | terminal.ansi.yellow / cyan / magenta |
| path | syntax.boolean → syntax.constant |
| fuzzy_accent | text.accent |
| accent_plan | modified |
| accent_remember, md_task_checked, diff_insert_fg | created |
| diff_delete_fg | deleted |
| diff_*_bg | created/deleted.background if distinct, else 22% mix of fg into bg |
| diff_equal_fg / diff_gutter_fg | syntax.comment / editor.line_number |
| selection_border / hover_border / prompt_border / prompt_border_active | border.selected / border.variant / border / border.focused; Borderless (alpha 0) falls back to element.selected / element.hover |
| accent_model | syntax.type |
| scrollbar_bg / scrollbar_fg | scrollbar.track.background / scrollbar.thumb.background |
| md_heading_h1..h4 / h5 / h6 | syntax.title / text.muted / syntax.comment |
| md_code | syntax.text.literal → string |
| link_fg | syntax.link_text → link_uri |

Deviation: h1–h5 are always bold (italic too if Zed's `title` is italic). Grok drops the `#` markers Zed shows, so regular-weight headings would be indistinguishable from body text.

## Syntax (`.tmTheme`)

Each Zed `syntax.<key>` becomes one TextMate rule (colour, `bold` if weight ≥ 700, `italic` if `font_style: italic`). The key→scope table is `SYNTAX_SCOPES` in the script, e.g. `keyword` → `keyword, storage.type, storage.modifier`; `function` → `entity.name.function, support.function, …`; `type` → `entity.name.type, support.type, …`; `property` → `variable.other.property, meta.object-literal.key, …`. Globals: background/foreground/caret/selection/lineHighlight from editor + players[0]. `markup.inserted/deleted/changed` ← created/deleted/modified.

Limits: Zed highlights via tree-sitter captures; syntect uses TextMate scopes, so grammars that don't emit a matching scope fall back to foreground. Kanso Mist is flat by design (one fill `#43464E` for hover/selection/code), so selected rows and visual selection share a colour, as in Zed.

## Sources

Colour values come from these Zed extensions (the Zed extension registry requires an OSI licence): Kanso, webhooked/kanso-zed; Kanagawa, ethangilmore/zed-kanagawa; JetBrains, artemevsevev/zed-theme-jetbrains.
