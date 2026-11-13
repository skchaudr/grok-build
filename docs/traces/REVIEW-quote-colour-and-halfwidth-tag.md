# Review: quote colour (760cda72) and half-width subagent tag (7c6e3879)

Code-only review, no cargo. Method: read the diffs, recomputed the visibility-test thresholds in Python, hand-computed the width-60 snapshot columns.

## 760cda72: md_quote for blockquote text and bar

**Verdict: land.** No blocking findings.

- `md_quote` is set in all 17 themes that `every_theme()` tests (groknight, grokday, tokyonight, rosepine, oscura, 12 zed ports). The 12 zed ports match the `MD_QUOTE` table in `scripts/themes/zed_port.py` value for value.
- parse.rs (+1), style.rs (+3), and playground_common.rs (+2/-1) are required, not scope creep. `MarkdownStyle` gained a `blockquote_inner` field, so every struct literal must set it. parse.rs is where the field takes effect on text inside a quote.
- zed_port.py is needed so regenerating zed_ports.rs does not drop `md_quote`.
- Bar and text both use `md_quote`: md_style.rs:133-134 and quote_bar.rs:44 (`quote_bar_style`) match.
- Tests: the new `quote_colour_is_distinct_and_readable_in_every_theme` (tokyonight.rs:~505) passes on the numbers below. Not run.

Not verified: no theme was looked at on screen. AGENTS.md requires a visual check in every shipped theme (`GROK_THEME=<name>`). Grokday's `#9e5408` is a dim brown at 4.86:1, the lowest margin in the table, so check that one first.

### Theme table (computed from source)

Contrast is md_quote against bg_base. Distance is the minimum RGB distance from md_quote to md_text, md_code, link_fg and md_muted. The test needs ≥4.5 and ≥60.

| Theme | md_quote | Body (md_text) | Contrast | Min dist |
|---|---|---|---|---|
| groknight | #e0af68 | #c8c8c8 | 9.21 | 102 |
| grokday | #9e5408 | #444444 | 4.86 | 109 |
| tokyonight | #e0af68 | #c0caf5 | 7.28 | 73 |
| rosepine_moon | #f6c177 | #e0def4 | 9.55 | 130 |
| oscura_midnight | #ebd96e | #e4e4e4 | 14.41 | 119 |
| kanso_mist | #dca561 | #c5c9c7 | 6.01 | 80 |
| kanagawa_wave | #d27e99 | #dcd7ba | 5.59 | 95 |
| kanagawa | #dca561 | #c5c9c5 | 7.47 | 72 |
| jetbrains_islands_dark | #cf8e6d | #bcbec4 | 6.42 | 90 |
| night_owl | #c792ea | #d6deeb | 7.62 | 76 |
| one_dark_pro | #c678dd | #abb2bf | 5.09 | 71 |
| nstlgy_dark | #cba6f7 | #ffffff | 8.18 | 103 |
| severance_dark | #ffcc80 | #40e7f6 | 12.83 | 172 |
| quasi_monochrome_dark | #ffd700 | #ffffff | 14.97 | 201 |
| jetbrains_islands_light | #a85c00 | #080808 | 5.00 | 151 |
| monosami_light | #82461e | #262626 | 5.81 | 98 |
| monosami_ink_light | #1e6469 | #121212 | 6.07 | 77 |

Terminal default uses `Color::Magenta` for md_quote and `Color::Reset` for body text. It is not in the RGB test set.

## 7c6e3879: subagent tag drops model before name

**Verdict: land.** No blocking findings.

- `tag_fit` (traces_follow.rs:640-647) matches the updated SPEC line (SPEC-subagent-rows.md:33). The model is dropped first, then the name is cut with `…`.
- Alignment holds. The tag is at most `tag_w - 2` wide, so `lead()` always pads by ≥2 spaces.
- Unlabelled children: `tag_text` gives an empty model, and `name(false)` returns the short model (line 103), so the tag is `↳ n haiku-4-5`. The model is kept, as the spec requires.
- `truncate_str` is the width-aware one from `render/line_utils.rs:44`, so the column math is in display width.
- Width-60 snapshots checked by hand (tag_w = 20, avail = 18):
  - `subagent_long_name`: `↳ 1 Audit the obs…` is 18 cols, then 2 spaces, then `started`. Matches.
  - `subagent_rows`, agent 1: `↳ 1 UI/UX lens` is 14 cols, then 6 spaces, then `started`. Matches.
  - `subagent_rows`, agent 2: `↳ 2 observability…` is 18 cols, then 2 spaces. Matches.

No fix needed.

## Summary

| Commit | Verdict | Fix needed |
|---|---|---|
| 760cda72 | land | none in code; on-screen check in each theme still owed (AGENTS.md) |
| 7c6e3879 | land | none |
