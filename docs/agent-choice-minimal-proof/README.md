# New-session agent picker in minimal screen mode: live proof

Verdict: **passed with one named non-blocking issue** (pre-existing, both modes, see below).

Host: khoj-38w. Binary: `~/.grok/bin/grok-fork` built from this commit
(sha256 prefix `f822fd70115c80ef`, 238880392 bytes). Previous binary kept at
`~/.grok/bin/grok-fork.52a7d02d`. Config: two `[[agent_choice]]` entries in
`~/.grok/config.toml` (`claude`, `pi`). Terminal: 120x36 tmux (`-L newfix`),
launched from `/tmp`. Every step is in `steps.log`; the tmux server was killed
after each pass.

## Bug (before)

`/minimal` then `/new`: the pane showed only the minimal status row and the
prompt. The picker was open (`MachinePicker::begin_new_session` returned `Ask`)
and captured every key, but `paint_picker` was only reached from the
fullscreen/inline render branches in `app_view.rs`; the minimal draw path
(`xai-grok-pager-minimal::live::draw_live`) never painted it.

## Fix

The open picker is now a prompt-replacing modal in the minimal live band
(`overlay::Modal::MachinePick`, resolved by `overlay::machine_pick_modal` ahead
of the per-agent modals, the same precedence `AppView::handle_input` routes
keys in). It is painted flush-left above the status line by
`agent_choice::paint_picker_inline`; the viewport is sized for it by
`compute_target`. Fullscreen/inline keep the centered popup (`paint_picker`,
now sharing the line builder).

## Captures

| file | shows |
|---|---|
| `01-minimal-after-new.txt` | minimal, `/new`: bordered "New session / Machine / > claude / pi / enter select · esc cancel" block, flush-left |
| `02-minimal-after-down.txt` | Down moves the cursor to `pi` |
| `03-minimal-after-esc.txt` | Esc closes the block; prompt and info row return |
| `04-minimal-after-enter-pi.txt` | Down, Enter: a new session is created (fresh welcome card, new session id) |
| `05`..`07-fullscreen-*.txt` | fullscreen unchanged: centered popup, Down moves, Esc closes |
| `08-minimal-pi-acp-process.txt` | process tree under the grok-fork pid before/after picking `pi` in minimal |
| `09-fullscreen-pi-acp-process.txt`, `09-fullscreen-after-enter-pi.txt` | the same control in fullscreen |

## Tests

- `xai-grok-pager` `app::agent_choice` (16, incl. new `inline_picker_paints_flush_left_with_every_choice_and_the_cursor`)
  plus new `app::app_view::tests::minimal_mode_routes_keys_to_the_open_machine_picker`: 17 passed.
- `xai-grok-pager-minimal` lib: 97 passed, incl. new
  `overlay::tests::machine_picker_replaces_the_prompt_in_minimal_and_paints_every_choice`.

## Named issue (non-blocking for this task, pre-existing, both modes)

Picking `pi` does not spawn `pi-acp` when `grok-fork` is launched plain (no
`--leader`). `Effect::CreateSession` sends `session/new` on the one existing ACP
connection and only stamps the choice into meta (`x.ai/agentCmd`,
`x.ai/agentName`, `app/effects/mod.rs::stamp_agent_choice`). The
`claude-agent-acp` adapter ignores that meta, so the new session is a second
`claude` process under the same adapter, labelled `pi` in the header
(`09-fullscreen-after-enter-pi.txt` shows the `pi /tmp` header; `08`/`09`
process trees show no `pi-acp` child). `scripts/picker-live.sh` runs with
`--leader`, where the leader honours the meta; that path was not re-tested here.
The picker UI (this task) is correct in both modes; the dispatch of the chosen
`cmd` in plain mode is a separate fix.

Not reachable in minimal: the dashboard `+ New Agent` path (minimal renders no
dashboard; `commit.rs` treats it as nothing to commit), so only `/new` was
proven there.
