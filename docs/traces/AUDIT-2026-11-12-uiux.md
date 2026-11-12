# grok-traces UI/UX audit, 2026-11-12

Scope: `grok-fork traces` as a TUI. Captured in a Rex session at 80 cols (`rex capture`, text and html), and in tmux panes at 200 and 40 cols (`tmux capture-pane`). The Rex session and tmux panes were killed afterwards. Every theme was measured through Rex html output. Colour was measured as WCAG contrast of each text span against its background, not eyeballed.

Code is current as of this audit. `docs/traces/TOOL-traces-modal.md` and `ISSUES-ux-audit.md` are stale: the standalone entry uses `board.rs` (ledger tabs) and `deck.rs` (follow), not the old `traces_modal.rs` List/Trajectory footers (`traces_modal.rs:4254` only runs when `board` is None, `traces_modal.rs:456`).

## Findings, ranked by how much they hurt

### 1. No way to change theme or any setting from inside traces

Evidence: the footer on the ledger is `f:follow │ e:subagents │ Tab:view │ c:compact │ r:resume │ y:copy resume │ /:search`. No settings key anywhere. Theme is only set in `~/.grok/config.toml` `[ui] theme` or through grok's own `/theme` command (`views/completion_dropdown.rs:229`), which lives in the agent TUI that standalone traces never starts.

Cause: `views/traces_modal/board.rs:364-456` (`handle_key`) has no settings branch. `views/traces_modal/standalone.rs` only opens by prefix. `settings/defs.rs` and `views/settings_modal/` exist and already hold `theme`, `max_thoughts_width` and others.

Closes it: add a key in `board.rs` `handle_key` that returns an `Action` opening `settings_modal`, add it to `render_footer` (`board.rs:815`), and route it in `app/dispatch/standalone_traces.rs`. Files: `board.rs`, `deck.rs` (footer), `standalone_traces.rs`, plus the settings modal entry point. Size: medium, 3-4 files.

### 2. Content stretches across the whole terminal (Sab's line-readability complaint)

Evidence at 200 cols, ledger: `◇ Live-feed-followups brief` sits at col 4 and `Claude · grok-build · 7h10m  12h` ends at col 200, with about 90 blank columns between them. Follow at 200 cols: the prompt band `› You are one of three auditors of grok-traces … The other      3:23 AM` runs to about col 190. The grok view itself, captured at 200 cols, has prose lines reaching about col 190, so the baseline Sab named has no measure cap either.

Cause: `board.rs:516` `let w = area.width.saturating_sub(4)` with no cap. `board.rs:721` pads the title-to-meta gap to the full width. `deck.rs:~218` sets the stream to `area.width - 4`. `traces_follow.rs:505` `let w = width - indent`, and `traces_follow.rs:531` wraps user text at `w - 4 - beside`. `ui_config.rs:226` defines `DEFAULT_MAX_THOUGHTS_WIDTH = 120`, but I found no reader of `max_thoughts_width` in the pager renderer (rg, excluding settings and config files). So the setting that should cap the measure is defined and never applied.

Closes it: one shared content-width cap (about 100-110 cols) applied in `board.rs` `render` and `deck.rs` `render`, with the meta kept next to the title. Also cap the follow body width in `traces_follow.rs`. Files: `board.rs`, `deck.rs`, `traces_follow.rs`. Size: small-medium. Decide first whether the grok view's measure is in scope too, since it is the stated bar.

### 3. Half width (40 cols) breaks the follow header and the ledger

Evidence at 40 cols, follow: rows 1-2 are blank. The title and the `● live` / `■ ended` word are gone, which is the only live/ended signal on screen. Ledger tabs: `traces  Live  Longest  Failed  Proje`, with `Harness` cut off. Rows drop harness and project: `◇ Prime agent harness refere…   5h`.

Cause: `deck.rs:278` `left_w = width - (rw + 2)` saturates to 0 when the right-hand meta is wide, and the right side is drawn only if `rw + 2 < width` (`deck.rs` header block, 253-297). `board.rs:775-800` clips the tab line with `set_line` and drops the chips. `board.rs` `row_lines` (664+) drops meta first when narrow.

Closes it: under about 60 cols, stack the follow header onto two rows (title row, then live word and meta) and keep the live word first. In the ledger, keep the tab row to the active tab plus a `…` for the rest. Files: `deck.rs`, `board.rs`. Size: small.

### 4. Footer hints are static and clipped

Evidence at 80 cols, ledger: `…│ y:` (hint cut mid-token). At 40 cols: `Tab:view │ /:` and `f:follow │ e:subagents │ Tab:view`. The footer always shows `r:resume` and `y:copy resume`, whatever the row. Follow footer at 40 cols: `Tab:next live │ v:grok view │ l:`. Grok view at 80 cols also cuts at `Ctrl+e:`, so clipping is inherited. There is no `?` help overlay (none found in the traces files).

Cause: `board.rs:815-826` and `deck.rs:299-315` are fixed `HintItem` lists. `shortcuts_bar.rs` clips at width without an ellipsis. The state-aware resume logic exists only in the legacy modal (`traces_modal.rs:4289`, `4318`, with tests at 5548-5607), so the live board ignores it.

Closes it: make the hints state-driven (reuse the `resume_action` logic from the legacy path) and drop whole hints by priority instead of clipping mid-token. Add `?` help. Files: `board.rs`, `deck.rs`, `shortcuts_bar.rs`. Size: small-medium.

### 5. Raw markdown in prompts and ledger summaries

Evidence, 200-col follow of the observability auditor: `  › You are one of three auditors … Your slice: **observability**.` and `## Sab's words (the owner)` on its own line. Ledger summary, 80 cols: `▸ I launched 3 Haiku auditors in the background: - **O…`.

Cause: `traces_follow.rs:529-545` (`Body::User`) uses `wrap()` on the raw text, not `self.md`. Agent text at `traces_follow.rs:553-575` does go through the markdown renderer, so only the prompt band is raw. `board.rs:755-760` (`split_at_width`) only swaps newlines. The summary text is the raw index string.

Closes it: render `Body::User` through `self.md` the same way as `Body::Text`. In `board.rs`, strip or render markdown in the summary before `split_at_width`. Files: `traces_follow.rs`, `board.rs`. Size: small.

### 6. Titles: junk and unstripped prefixes

Evidence: a ledger row titled `==============================…` (a banner line), and `[Image #1] read the current pane or th…`.

Cause: `traces_modal.rs:4421-4440` (`clean_name`) strips leading `#*>"'` and wrapper tags only. Lines of pure punctuation and `[Image #N]` prefixes pass through.

Closes it: skip punctuation-only lines and `[Image #N]` markers in `clean_name`, and take the first line with letters. Files: `traces_modal.rs` (plus the title source in `grok-traces` if the index is the right place; guessing which). Size: small.

### 7. Tab names do not match their content

Evidence: tab `Longest` shows the group `▾ Most active time 629`. Tab `Projects` shows `▾ grok-build 60`. The active tab is marked only by colour and bold (`rgb(…) bold` in html, plain text in `rex capture`), so a no-colour terminal cannot see which tab is active.

Cause: `board.rs:55` (`"Harness"`), `board.rs:50-56` (`VIEWS`), `board.rs:252` (`"Most active time"`), `board.rs:775-788` (active tab styled only).

Closes it: rename the group to match the tab (`Longest`), and add brackets or an underline to the active tab. Files: `board.rs`. Size: small.

### 8. Ended sessions open grok's view with no traces context

Evidence: Enter on an ended local row shows only `main /V/M/o/repos/grok-build  [Dashboard]` at the top and a `[Dashboard]` tag. No title, harness, or model, so you cannot tell it came from traces.

Cause: `standalone_traces.rs:87-94` (`open_claude_in` → `open_feed_in`) and `standalone_traces.rs:395` (`read_only_view`) reuse the agent view with no traces header.

Closes it: a one-line header inside the read-only view showing source, model and project. Files: `standalone_traces.rs`, plus the agent view header. Size: medium.

### 9. Hook errors shown as failed tool calls (minor, data-side)

Evidence: `◆ Bash  Check traces bin target, c…  ✗ PreToolUse:Bash hook error: (Recur…`.

Cause: guessing. The index appears to classify a hook error as a tool error. Not traced into the index code. Out of my slice; passing it on.

## Per-theme table

Measured with Rex html on the ledger. Columns: min text contrast; spans under 4.5:1 out of text spans; any selection-band issue. Contrast is measured against each span's own background. Terminal-rendered truecolor only, so 256-colour terminals may look different (`requires_truecolor`, `theme/mod.rs:~140`). `auto` was not tested (it resolves from OSC 11).

| Theme | Distinct and readable? | Min contrast / under 4.5 | Worst problem |
|---|---|---|---|
| groknight | partly | 2.18 / 24 of 39 | Metadata `Claude · grok-build` at 2.18:1 |
| grokday | partly | 1.83 / 28 of 44 | Tool names such as `SendMessage` at 1.83:1 |
| tokyonight | no | 1.37 / 30 of 48 | Metadata nearly invisible |
| rosepine-moon | no | 1.19 / 35 of 52 | Metadata at 1.19:1 |
| oscura-midnight | mostly | 3.14 / 22 of 49 | Nothing under 3:1, but most text under 4.5 |
| kanso-mist | partly | 2.16 / 32 of 49 | Metadata; selection only +1.39:1 |
| kanagawa-wave | partly, near-twin of kanagawa | 1.88 / 32 of 49 | Same background and selection as kanagawa |
| kanagawa | partly, near-twin of kanagawa-wave | 2.04 / 32 of 49 | Same background and selection as kanagawa-wave |
| jetbrains-islands-dark | partly | 2.01 / 32 of 50 | Metadata |
| night-owl | partly | 3.01 / 30 of 47 | Metadata |
| one-dark-pro | no | 1.64 / 33 of 57 | Metadata and `The buildability…` text |
| nstlgy-dark | partly | 1.96 / 23 of 50 | Metadata |
| severance-dark | yes, best dark | 3.11 / 9 of 50 | Selection clearly distinct (2.09:1) |
| quasi-monochrome-dark (configured here) | partly | 2.30 / 22 of 51 | Metadata; identical numbers to terminal |
| jetbrains-islands-light | partly | 2.81 / 31 of 56 | Metadata |
| monosami-light | no | 2.72 / 31 of 49 | Selected row has the same background as the page (228 on 228). Only the `▏` bar marks it |
| monosami-ink-light | yes-ish, best light | 2.94 / 2 of 48 | Metadata still under 3:1 |
| terminal | partly | 2.30 / 22 of 48 | Identical to quasi-monochrome-dark, so near-twin |

Judgement on "distinct": terminal and quasi-monochrome-dark have identical measured backgrounds and selection colours (0,0,0 and 51,51,51), and kanagawa and kanagawa-wave share background and selection values. These are near-twins by measurement; I did not eyeball them.

## Not verified

- Visual look of each theme (only measured, not screenshots compared).
- `auto` theme.
- 256-colour terminals.
- Whether the grok view's own measure is meant to be capped (see finding 2).
