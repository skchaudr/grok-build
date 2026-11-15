# Queue: changes to make before the next install run

One build cycle ships everything under `## Queued`: targeted `--release` tests, then `scripts/install-fork.sh`. An item is queued only after a scout mapped it and a reviewer validated its size and scope.

Entry format:

```
### <short name>
- Ask: <Sab's words, dated>
- Scope: <files, rough size>
- Evidence: <scout doc> / <review doc>
- Needs: <tests to add or regenerate; anything outside this repo>
```

## Queued

### Footer shows status, not keybinding hints (cache, Claude usage first)
- Ask: Sab 2026-11-14, "4 useless keybindings that `?` and `Ctrl-,` easily overrule in terms of value ... the five hour percentage for Claude should go at that footer right there. Same with Codex and same with Open Code Go Even Grok". Sab 2026-11-15, "Its even worse: y : copy Enter: open j/k:nav Shift+l/h:turn Ctrl+x: shortcut. There is the input editr footer and then navigational one where it shows defaults. I already have a command palette, what I do not have is a warm cache signal. Why not include cache, claude usage, code, opencode"
- Decided by those words: both footers (typing and scrollback navigation) stop showing default-key hints; the command palette and shortcuts help cover them. The row carries cache warmth, then Claude 5h/7d, then Codex and OpenCode as each gets a source.
- Slices in build order (all pager-only; reviewed in `docs/review/status-footer-scope-2026-11-15.md`; scouts `docs/SCOUT-footer-hints-removal-2026-11-15.md`, `docs/SCOUT-cache-warmth-2026-11-15.md`, `docs/SCOUT-usage-footer-areas-2026-11-15.md`):
  1. Hint trim: add `state: bool` to `HintItem` (`views/shortcuts_bar.rs`), set it on the hints that must stay (CancelTurn, Interject, send to bg, queue/send now, accept suggestion, `q/Esc:back`, `Space:prompt`, kill, cards/viewers), `retain(|h| h.state)` at `app/agent_view/render.rs:661-668`, drop the help hint at `:3889-3902`. About 40 lines. Re-record the 5 footer snapshots.
  2. Status slot: delete dead `right_text` in `views/shortcuts_bar.rs`, add `with_status` painted first from the left, pass it from `render.rs:3884-3903` on the Pane path only. About 40 lines + half-width test.
  3. Claude 5h and 7d in the slot, from the live `rate_limits` in `views/side_panel.rs:58`, statusline file only as fallback. About 20 lines.
  4. Cache age ("cache 12m" since the last response with cache read/write, hidden before one): read `response.usage.cached_read_tokens/cached_write_tokens` before the Claude filter at `app/dispatch/prompt.rs:1832-1836`, stamp a new field (not `last_active_at`, which aborted turns also set). About 25 lines. Shows age only, no warm/cold word.
- Reference Sab pointed at 2026-11-15 ("look at the DSH TUI Claude"), captured from the `dsh-tui` tab on sab-air: under each turn `↑10 · ↓4.7k · cache read 370.9k/write 16.5k · 1m 2s`; footer `claude-fable-5-2 … cache 98.…% … ctx 8.1% (81k/1.0m)` and `? for shortcuts` as the only key hint. Slice 4 should show the cache hit share and the per-turn read/write line the same way, from the counts the pager already receives and drops (`app/dispatch/prompt.rs:1832-1836`), in addition to age.
- Needs: footer render test at full and half width in every theme (`GROK_THEME=<name>`), looked at on screen, typing and navigation footers both. Sab to answer: status row in shortcuts bar vs existing status-line row; keep `Space:prompt` and `q/Esc:back`; keep history/scrollback search hints; help hint removed entirely (my picks in the review).

## Being scouted

(none)

## Flagged, not scouted

### Grok sessions in the resume picker
- Ask: Sab 2026-11-15, "grok-fork apparently cannot resume normal grok sessions, which is kind of lame, in fact i do not know on sab-air if any sessions are currently resumable"
- Scouted: `docs/SCOUT-grok-resume-2026-11-15.md`. Plain `grok-fork` lists Grok sessions (8 of 21 on the Mini; the 13 hidden ones have 2 lines of history or fewer). `grok claude` lists only Claude sessions, because it asks only the Claude adapter (`app/effects/mod.rs` external branch, `dispatch/session/foreign.rs:65`). sab-air has 21 Grok sessions on disk. Loading a session was not exercised.
- Open: showing Grok rows under `grok claude` is a listing change, but opening one there means switching the running agent from Claude to Grok; that part is not mapped.

### Slash commands in grok claude go through the model
- Ask: Sab 2026-11-14, "If I want to make a deterministic change like changing effort level, it HAS to go the model first, it gets queued"
- Known: `docs/HARNESS-FACTS.md`; the settings rows already change effort with no model turn. Where a typed `/effort` gets forwarded as prompt text is not traced.

### 7d usage has no fallback
- Seen on screen 2026-11-14: F3 Info shows "7d usage not reported" until Claude sends it live; the statusline file has a 7d figure the panel does not read (`views/side_panel.rs`, `claude_five_hour_pct`). Likely absorbed by the footer usage change.

### Pi feature rows show under Claude
- Seen on screen 2026-11-14: Settings, Agent tab under `grok claude` still lists "Pi features" (Ask-Question timeout, Pi config skill, Pi resources). Those rows are not marked Pi-only.

### Footer snapshot tests depend on the terminal
- Three side-panel snapshots (`closed`, `narrow_terminal_unfocused_panel_hides`, `open_unfocused_lets_the_chat_type`) expect `Ctrl+.:shortcuts`; run inside Rex they render `Ctrl+x:shortcuts` (`actions/defaults.rs`, `terminal/mod.rs` `kitty_skip_reason`). Will be touched anyway if the footer changes.

### One tailored TUI per host
- Sab 2026-11-15: "I am surprised we are sticking to ONE TUI for grokclaid. Like grok claude could have a full claude talored settings modal, grok pi could have its own, prime agent has its own, which we will get to eventually, recent changes were made for prime"
- Today: host profiles exist for Pi and Claude (`install_pi_host_ui`, `install_claude_host_ui` in `xai-grok-pager-bin/src/main.rs`; `HostFeatureManifest` in `xai-grok-shell/src/host_features.rs`). They set brand, welcome and changelog; settings are one shared modal with per-host rows shown or hidden. Prime has rendering and resume fixes (`caec749c`, `2f969e3b`) and no host profile.

### grok claude shows nothing while Claude is thinking
- Ask: Sab 2026-11-15, "grok-claude does this thing where it no longer shows anything while claude is just thining".
- Not scouted. Related: `7449ac53` (Claude host working state after Worked-for).

### install-fork.sh does not ship grok-pi-acp
- Sab 2026-11-15, on the fact below: "That s an issue".
- Fact: `scripts/install-fork.sh` copies `grok-fork` and `grok-traces` only. On sab-air `~/.grok/bin/grok-pi-acp` is from 2026-11-09 while `grok-fork` is from 2026-11-14, so `grok grok-pi` runs a new pager against an old Pi piece.

### Pi on sab-air: what Sab runs is Dwsy's binary
- Checked on screen 2026-11-15: `pig`, `grok-pi`, `pi-grok` on sab-air are Dwsy's standalone 0.1.13 (own home `~/.grok-pi`); ours runs only as `grok grok-pi`. Report: `docs/INVESTIGATE-grok-pi-regression-2026-11-15.md`.
- Sab 2026-11-15, in Dwsy's binary: "I'm not able to dispatcher start a new Pi session if Pi is mid turn, which sucks". Not ours to fix unless the same happens in `grok grok-pi`; not checked there.

### Pi notices in the scrollback are out of reach (grok grok-pi)
- Ask: Sab 2026-11-15, on the lines "Usage cleanup started", "daily-memory: session note dropped", "Switched to GPT-6.1 Sol": "all this changelog and notification stuff, just being out of reach, not expandable, or engaged with, sucks". Same message: "grok grok-pi is coming together well, the one significant complaint as of right now".
- Fact: `pi/ui/notify` info messages are pushed as plain system lines (`app/acp_handler/pi_ui.rs:29-47`). The same messages are kept in `agent.pi_notifications` and `/notify` already opens a list with search, expand and copy (`app/dispatch/router.rs:1630`, `app/agent_view/pi_tree.rs:574`), but nothing on the line leads there.
- Not scouted: making the lines themselves selectable/expandable in scrollback navigation; the changelog part.

### grok grok-pi dies at startup when the Pi workflows feature is on
- Seen on sab-air 2026-11-15 07:26: `Error: unable to receive 'initialize' response` and `server shut down unexpectedly`.
- Cause: `pi_workflows = true` in `~/.grok/config.toml` makes `grok-pi-acp` load its bundled workflows extension, which registers the same `workflow` tool as the installed Pi package `npm:@osolmaz/pi-workflows`; Pi exits 1. Tested against a throwaway copy of the home with the installed (2026-11-09) `grok-pi-acp`: Dwsy's `native-feature-conflicts.toml` and a `[pi.resources] block` both still fail; `pi_workflows = false` starts.
- Done on sab-air: `pi_workflows = false` (backup `~/.grok/config.toml.bak-20261115-0730`); `grok grok-pi` seen starting in `~/repos/acpx`.
- To fix in code: the clash must not kill startup (skip the bundled feature or block the package, and say so on screen); the pager must show Pi's error instead of a bare channel failure. Re-test with the 2026-11-14 build, which has the resource policy the installed one lacks.

### Model picker shows the provider twice per row (grok grok-pi)
- Ask: Sab 2026-11-15, on rows like "GPT-6.1 Sol · openai · openai": "you don't see the duplication and repetition, OpenAI, OpenAI ... there's just weird duplication".
- Cause: `crates/codegen/xai-grok-pager/src/slash/commands/model.rs:334-346` builds the label as `"{label} · {provider}"` and then also sets the row description to the provider, so the picker prints it twice.
- Change: keep the provider once (drop it from the description or from the label); about 2 lines plus the tests near `:856`.

## Not queued

- Fresh Claude usage via adapter (`get_usage` at session start and turn end): RESCOPE. Needs Sab's call on where the `~/.dsh` patch lives, a shape conversion (0-100 and ISO vs 0..1 and epoch), and a `usage_update` that does not zero the context meter; conflict with patch 0002 untested. Review: `docs/review/usage-footer-scope-2026-11-15.md` F6.
- Codex 5h/7d in the footer: RESCOPE. Source is real (sab-air rollout `payload.rate_limits`, `used_percent` + `resets_at`) and it is account-level, not tied to the `grok claude` session. The newest figures (2026-11-10/11) have `resets_at` in the past, so show it only when `resets_at > now`, else hide. About 60 lines, off the UI thread; low value on days Codex is unused. Review: `docs/review/status-footer-scope-2026-11-15.md` F6.
- OpenCode in the footer: DROP (sab-air checked 2026-11-15). `opencode` 1.19.4 exists there, but `opencode.db` has 700 messages, newest 2026-10-20, cost 0.0, no plan percentage in db, config, auth or logs. Tokens from three weeks ago is not a status. Reopen only if Sab names a place the OpenCode Go quota can be read. F7.
- Cache warm/cold and minutes left: RESCOPE. The SDK TTL is not on the wire; the adapter must forward `cache_creation.ephemeral_1h_input_tokens/ephemeral_5m_input_tokens` (about 8 lines in `acp-agent.js` near `:3437` and `:6530`) and Sab decides where that adapter patch lives (same open call as fresh usage). F1.
- Grok in the footer: `usage_pct` (`credit_bar.rs`) is a credit allowance, not a 5h window; needs its own label if shown.
