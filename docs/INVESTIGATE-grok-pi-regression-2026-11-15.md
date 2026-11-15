The version Sab calls worse is most likely Dwsy's standalone 0.1.13, not the fork's copy: on sab-air, `pig`, `grok-pi` and `pi-grok` have always resolved to Dwsy's standalone binary in `~/.local/bin` (the hub `pig` script that runs the fork is not on PATH), and that binary jumped from 0.1.11 (Oct 25) to 0.1.13 on Nov 13 10:44; the fork's copy only runs through `grok grok-pi`, which he used 0 times in history.

Investigation 2026-11-15, read-only on sab-air and the Mini. Nothing started, resumed, installed or built. No prompt text copied.

## A. What Sab actually runs

| finding | evidence | effect on the user | certainty |
|---|---|---|---|
| PATH has `~/.local/bin` (line 3) and `~/.grok/bin`; `~/.grok-hub/scripts/hub` is not in PATH. `pig`, `grok-pi`, `pi-grok` all resolve to `~/.local/bin/*`. | `zsh -ic 'type pig grok-pi pi-grok'` on sab-air: all three under `/Users/sab-mini/.local/bin`; PATH listing shows no `grok-hub` entry | Typing `pig` or `grok-pi` has never reached the fork's Pi. Commit `ab8cfd6d` (2026-11-10, "pig: run grok-pi in the fork pager") changed only `~/.grok-hub/scripts/hub/pig`, which Sab's shell never calls. | confirmed |
| `~/.local/bin/grok-pi` is `0.1.13 [stable]`, 75,310,952 bytes, mtime Nov 13 10:44; `pig` and `pi-grok` are symlinks to it with the same mtime. | `ls -la ~/.local/bin`, `grok-pi --version` | Everything run via `pig`/`grok-pi` since Nov 13 10:44 is Dwsy 0.1.13. | confirmed |
| Before Nov 13 10:44 the same entry points ran Dwsy 0.1.11 `[stable]`, built Oct 25. | `docs/grok-pi-fold-scout.md` "Installed binary" line (an agent wrote it, 2026-11-08, from `sab-air:~/.local/bin/grok-pi`) | The 0.1.11 binary is gone (no backup in `~/.local/bin`). The version Sab liked was 0.1.11; the one he runs now is 0.1.13. | inferred (agent note; no backup to check) |
| Shell history on sab-air (`~/.zsh_history`, commands starting with each word): `pig` 11, `grok-pi` 25, `grok pi` 4, `pi-grok` 0, `grok grok-pi` 0. Newest `grok-pi` 11-15 06:07, newest `pig` 11-15 02:26, newest `grok pi` 10-22 03:13. Since 11-10: `pig` 11-10 23:52, 3x 11-12 (13:45), `grok-pi` 3x 11-12, 2x 11-13 (10:43), 2x 11-15. | python count over `~/.zsh_history` | Sab's Pi use is Dwsy standalone, every day this week. The fork's `grok grok-pi` has zero history lines ever; `grok pi` (the older patched pi-acp adapter in grok-fork) last used Oct 22. | confirmed for typed commands; sessions started from menus or scripts are not in this count |
| Dwsy standalone logs to `~/.grok-pi/logs/unified.jsonl`: 205 lines since 11-09, 1 session id, newest line 11-15 14:07Z (= today 06:07 local, matching the newest `grok-pi` history line). | python over `~/.grok-pi/logs/unified.jsonl` fields `ts`, `sid` | Confirms 0.1.13 was started today. | confirmed |
| Pi session files modified in the last 7 days: 27 session headers (type `session`, version 3), 43 `.jsonl` files total including subagent files, newest 11-15 06:10. Cannot be split by launcher: all four paths write to `~/.pi/agent/sessions`. | `find ~/.pi/agent/sessions -mtime -7` | No per-path session count is possible from these files. | confirmed that attribution is not possible |
| Fork pager log `~/.grok/logs/unified.jsonl`: 12,782 lines, 14 session ids since 11-11, all agents mixed (`ver` 1.0.62 / 1.0.45), no agent column. | python over fields `ts,src,pid,ver,lvl,msg,ctx,sid` | Cannot tell how many were Pi. | confirmed that attribution is not possible |

## B. What differs between the version he runs now and the one he liked

Three candidates: (1) Dwsy 0.1.11, Oct 25, the likely "liked" one; (2) Dwsy 0.1.13, installed Nov 13, what `pig` runs now; (3) the fork's copy, `grok grok-pi`, which he does not use.

| finding | evidence | effect on the user | certainty |
|---|---|---|---|
| Default model in Dwsy's home config changed: `default = "xai::grok-4.7"`, `default_reasoning_effort = "high"`. The Oct 25 backup has `opencode-go::kimi-k3` with `"max"`. | `diff ~/.grok-pi/config.toml.bak-20261025 ~/.grok-pi/config.toml` (secrets filtered; config.toml last written Nov 15 06:07) | Every new `pig` session starts on a different model with lower reasoning effort than the Oct 25 config. The file was written by the app or by Sab; the diff does not say which. | confirmed diff; who wrote it is unknown |
| Same diff: `theme` `groknight` to `pi:titanium`; `psm_resume_index` false to true; `review_include_reads` false to true; new `[ui.contextual_hints] plan_mode = false`; new `[pi.resources] block = ["npm:@osolmaz/pi-workflows"]`. | same diff | Different look, different resume behaviour and review content, and a blocked workflow package. These match "settings, config all felt different". | confirmed diff |
| Extension load failure on standalone: 23 lines `Error: Failed to load extension ".../T/pi-grok-feature-pi_workflows..."` with the hint `Start without extensions using "pi -ne"`. | `rg` over `~/.grok-pi/logs/pi-rpc-stderr.log` (1,647 lines) | The pi-workflows feature extension fails to load on 0.1.13 startup (23 times in this log). This also fits slash-command differences (workflow commands missing). The `block` entry above may be an attempt to stop it. | confirmed errors; link to missing commands is inferred |
| 986 of the 1,647 stderr lines are `Warning: No models match pattern "cursor/..."` (for example `cursor/composer-2.5:fast`, `cursor/auto`). | same stderr log | The cycle-model list in 0.1.13 names `cursor/*` models that Pi cannot resolve. The model picker/cycling is thinner than it was. | confirmed lines; effect on the picker is inferred |
| Dwsy standalone isolates state in `~/.grok-pi` (own `config.toml`, hooks, skills, logs); `grok-pi --help` lists `migrate-home` ("Copy allowlisted state from stock Grok home `~/.grok` into `~/.grok-pi`"). The fork's `grok grok-pi` reads `~/.grok` (shared with Claude and normal Grok; the hub `gb` wrapper). | `grok-pi --help`; `docs/grok-pi-fold-scout.md` (agent note) "`home.rs` isolation conflicts with grok-fork sharing `~/.grok`"; `ls ~/.grok-pi` | Two different config homes: settings changed in one never show in the other. Anyone who tried `grok grok-pi` and then `pig` would see different config, which fits his sentence. | confirmed layout; the cross-effect is inferred |
| Slash commands: Dwsy's standalone registers `/tree`, `/review`, `/pi-config`, `/pi-models`, `/plan-mode`, `/reload`, `/clone`, `/hotkeys`, `/notify` and maps Pi `get_commands` into the registry (`PI_GROK_NATIVE_COMMANDS`). In the fork the adapter crate is present but the fork pager hides commands by agent command string (`acp/mod.rs:469-520`). | `docs/grok-pi-fold-scout.md` Slash row (agent note); repo has `views/pi_config.rs`, `views/pi_models.rs`, `views/pi_settings/*` | The views exist in the fork crate; whether `grok grok-pi` shows them is inferred from code, not seen on screen. I did not start a session. | inferred |
| Agent dashboard: standalone has `views/subagent_catalog_pane.rs` and 23 bundled `pi-grok-*` extensions (subagents 4,670 lines, bash 5,360); fork has `crates/codegen/xai-grok-pager/src/views/dashboard/` (Grok's own) and `subagent_catalog_pane.rs`. | `ls crates/codegen/xai-grok-pager/src/views`, `ls extensions` | The two dashboards are different code paths; I cannot say from reading which looks worse. Needs a screen look on sab-air. | inferred; not seen |
| Binary size: standalone `grok-pi` 75 MB, fork's `grok-pi-acp` 21 MB (stdio agent only, no TUI) inside `grok-fork` 201 MB. | `ls -la ~/.local/bin ~/.grok/bin` | The fork's copy has no own TUI or launch args (`--model`, `--session`, `--fork`, `--continue` of the standalone). It is a different product shape. | confirmed sizes; effect inferred |

## C. Failures on record

| finding | evidence | effect on the user | certainty |
|---|---|---|---|
| Standalone 0.1.13 stderr: 532 x `[dpi-rpc] stream: fetch failed; reconnecting`, 42 x `no data for 45s; reconnecting`, 16 x `stream: terminated; reconnecting`, 6 x `[dpi-rpc] nf...`. | `~/.grok-pi/logs/pi-rpc-stderr.log` (file ends Nov 14 05:27) | The durable-agent (dpi) stream drops and reconnects hundreds of times. A visible symptom would be the agent dashboard going stale or flickering. | confirmed counts; user-visible symptom inferred |
| Standalone: 23 x pi-workflows extension load failure (see B). | same log | Startup noise, workflows unavailable. | confirmed |
| Standalone unified log since 11-09: 2 x `prompt.ack_soft_notice` (warn) on 11-09. No errors. | `~/.grok-pi/logs/unified.jsonl` | Clean apart from that. | confirmed |
| Fork pager log since 11-11: 19 x `auth.sleep.gate_set`, 14 x `prompt.ack_soft_notice`, 10 x `prompt.ack_timeout` (first 11-12 23:16), 6 x `model catalog refresh failed` + 6 retries (first 11-12 05:18), 1 x `startup phase over budget` (11-13 17:15). All agents mixed. | `~/.grok/logs/unified.jsonl` | Cannot assign to Pi. `prompt.ack_timeout` x10 is the only hang-shaped entry. | confirmed counts; attribution unknown |
| No `grok-pi-acp` stderr log exists on sab-air. | `ls ~/.grok/logs ~/.grok-pi/logs` | Failures of the fork's Pi host are not recorded anywhere I could read. Absence of errors proves nothing. | confirmed |

## D. Drift

| finding | evidence | effect on the user | certainty |
|---|---|---|---|
| The fork's adapter is taken from Dwsy branch `719ba57f` (xAI sync `2bdd1d6a`, 2026-10-13), 37 commits behind Dwsy `upstream/main` at the time of the scout (2026-11-08). Adapter crate `pi-grok-adapter` version 0.4.0, added to the fork in `6246c4f3` (2026-11-09). | `docs/grok-pi-fold-scout.md` section 1 (agent note); `git show --stat 6246c4f3`; `crates/codegen/pi-grok-adapter/Cargo.toml:3` | The fork's copy is older than Dwsy 0.1.13 by at least those 37 commits. | agent note plus commit; branch contents not re-read |
| The repo's `CHANGELOG.MD` top entry is `0.1.11-beta.1` (2026-10-13). | `CHANGELOG.MD:9` | The fork carries no 0.1.12 or 0.1.13 changelog. Dwsy 0.1.13 shipped after it. | confirmed |
| The fork's own `bin/grok-pi.rs` (62.6 KB) exists in the repo but sab-air runs `grok-pi-acp` through the picker, not that bin. | `ls crates/codegen/xai-grok-pager-bin/src/bin`; `gb` lines 72-84 | The fork's standalone `grok-pi` binary is not installed on sab-air. | confirmed |
| Dwsy 0.1.11 to 0.1.13: what changed in Dwsy's code is not in this repo. Not found, looked in: `CHANGELOG.MD` (stops at 0.1.11-beta.1), `docs/upstream/UPSTREAM_CHANGELOG.md` (tracks xAI syncs only), `~/.grok-pi` on sab-air, `npm ls -g` (no `@dwsy/grok-pi`). No Dwsy checkout on either machine. | rg/ls above | The release notes for 0.1.12/0.1.13 would show what changed in settings and slash commands. | not found |

## What Sab would notice

1. Different default model and lower reasoning effort in `~/.grok-pi/config.toml` (grok-4.7 / high instead of kimi-k3 / max). Felt on every session.
2. A different theme (`pi:titanium`) and different resume and review behaviour (`psm_resume_index`, `review_include_reads`): "settings and config felt different".
3. Missing workflow slash commands: the pi-workflows extension fails on every start (23 errors) and is also blocked in config.
4. Agent dashboard going stale: 590 dpi stream fetch failures, timeouts and terminations in one stderr log.
5. A second, separate home: anything he set up under `~/.grok` (what `grok grok-pi` reads) does not exist under `pig`.
6. If he did try `grok grok-pi`: no standalone flags, an older adapter (37+ commits behind Dwsy), and no log to explain failures.

## Options

| option | what it changes | what it loses |
|---|---|---|
| **A. Put back the 0.1.11 behaviour on `pig`: restore the Oct 25 config values** (`default` model, `theme`, the two flags, remove the `block`) in `~/.grok-pi/config.toml` | Fixes items 1 to 3 without a new binary | The workflow extension still errors; the 0.1.11 binary is gone, so only config is restored |
| B. Pin `pig` to the older Dwsy release | Back to the version he liked | Needs the 0.1.11 binary; no backup exists, Dwsy GitHub Releases would have to be re-downloaded |
| C. Point `pig` at the fork (`grok grok-pi`) | One house | Older adapter, no standalone flags, no fork-Pi logs; he has not used it once |

Pick: A, then ask Sab which version he actually liked (0.1.11 or the fork) before anything else; I did not see either on screen.
