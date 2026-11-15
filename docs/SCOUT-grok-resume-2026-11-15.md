# Scout: which sessions `grok-fork` can resume (2026-11-15)

Read-only scout for the brief `docs/briefs/scout-grok-resume-2026-11-15.md`. No source edits, no cargo, no installs, no model turns. Rex tabs used for the TUI checks were closed.

## Result by host

| host | lists Grok sessions | lists own sessions | loads | evidence |
| :--- | :--- | :--- | :--- | :--- |
| plain `grok-fork` (Grok host) | yes: 8 rows under group `repos-grok-build` | yes (the same 8 rows) | not tested (Enter is forbidden in this brief) | Capture 1 below; code path in "Q3" |
| `grok claude` (Claude host) | no: no Grok-host title appears in a full scroll of the Claude list | yes: Claude sessions grouped by project (`repos-grok-build`, `Users-sab-mini`, `tmp-was-research`, `wt-grok-build-gb-bb-traces-scout-claude`, ...) | not tested | Captures 2 and 3 below |
| Pi host | not tested (no Pi launch in this scout) | not tested | not tested | `pi_catalog_host` at `crates/codegen/xai-grok-pager/src/app/dispatch/session/load.rs:1882` |

**Plain `grok-fork` does list Grok sessions.** It is not broken at the listing step. The "cannot resume" symptom reported by Sab on 2026-11-15 is therefore one of two things, and this scout did not confirm either by loading a session:

1. Under `grok claude`, the picker shows only Claude sessions by design (commit `eca9236f`). Grok sessions are not requested from that host.
2. Under plain `grok-fork`, 13 of the 21 sessions on disk are missing from the picker (details in Q3).

## Counts

- Mac (`~/.grok/bin/grok-fork sessions list`): 21 data rows. All 21 have `cwd` `/Volumes/Mini-SSD/offload/repos/grok-build`. All 21 have a matching directory under `~/.grok/sessions/`.
- Mac picker (plain `grok-fork`, ctrl+r): 8 rows. The 13 missing rows have `chat_history.jsonl` of 2 lines or fewer.
- sab-air (`sessions list`, read-only over ssh): 21 data rows. The brief said 22 from an earlier check; the measured count now is 21. All 21 have a matching directory under `~/.grok/sessions/`. `~/.grok/sessions/` has 98 entries (per-cwd directories), 16,621 files at depth 3.
- Binary: `~/.grok/bin/grok-fork` is identical on both machines (sha256 prefix `85e9fcab4357fd5e`). It reports build `20deef38`, which contains `eca9236f` (verified with `git merge-base --is-ancestor`).

## Captured screens

Capture 1, plain `grok-fork`, ctrl+r (Mac, Rex tab):

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  Resume session                                                           [✗]│
├──────────────────────────────────────────────────────────────────────────────┤
│ / to search                                                          Grok f  │
│──────────────────────────────────────────────────────────────────────────────│
│ repos-grok-build ────────────────────────────────────────────────────────────│
│  › Resume Claude session grok-build Sonnet 5.5              20h ago · 9 msgs │
│  › Grok-Pi Software Engineering Session                     1d ago · 31 msgs │
│  › Simple Hello Greeting from User                          2d ago · 12 msgs │
│  › Resume Claude Session 43d0aff9                           3d ago · 54 msgs │
│  › Preserve attached sessions after Grok repo deletions    3d ago · 424 msgs │
│  › Resume Claude Session 40e362e8                           6d ago · 25 msgs │
│  › Execute PACKET-learn-all on main                       24d ago · 331 msgs │
│  › Removed Claude sab-air LiteLLM base URL                 26d ago · 73 msgs │
└──────────────────────────────────────────────────────────────────────────────┘
```

The filter label at the top right is cut off in the capture ("Grok f"). I could not read it: the brief allows only ctrl+r, arrows and escape, so `f` was not pressed.

Capture 2, `grok claude`, ctrl+r, first screen (Mac, Rex tab):

```
│ repos-grok-build ───────────────────────────────────────────────────────────█│
│  › Scout usage footer areas brief                                  just now  │
│  › Scout Grok resume brief                                         just now  │
│  › Packet-review.md landing review                                   1m ago  │
│  › Claude usage freshness brief                                     51m ago  │
│  › reply with the single word pong                                   7h ago  │
│  › /resume                                                          10h ago  │
│  › Review-quote-and-tag brief                                       10h ago  │
│  › Rex image paste brief                                            10h ago  │
│  › Grok TUI resume showing no sessions                              17h ago  │
│  › Read docs/briefs/claude-settings-modal-2026-11-14.md and carr…   21h ago  │
│  › Read docs/briefs/review-claude-host-branding-2026-11-14.md an…   21h ago  │
│  › Herdr Grok traces TUI audit                                       1d ago  │
│  › Rex image paste brief                                             1d ago  │
```

Capture 3, `grok claude`, scrolled to the `Users-sab-mini` and `tmp-was-research` groups (arrows only):

```
│ Users-sab-mini ─────────────────────────────────────────────────────────────█│
│  › Delta brief script                                                4d ago █│
│  › AeroSpace repeated kill attempts                                 10d ago  │
│  › /resume                                                          11d ago  │
│  › Taskdog task #18: Rainwater page is looking bare and could us…   24d ago  │
│ tmp-was-research ─────────────────────────────────────────────────────────── │
│  › Read-only web research. Find PUBLIC evidence that "Water and …    9d ago  │
│  › Research task (read-only web research, do not sign up or post…    9d ago  │
│ wt-grok-build-gb-bb-traces-scout-claude ──────────────────────────────────── │
│  › Gb-bb-traces-scout-claude cell brief                             21d ago █│
```

None of the 8 Grok-host titles appear in any of the scrolled `grok claude` captures.

## Q3: code, per host

**Listing fetch.** `Effect::FetchSessionList` is handled in `crates/codegen/xai-grok-pager/src/app/effects/mod.rs`, around line 789.

- Native branch (plain `grok-fork`, `external_agent == false`): sends `x.ai/session/list` with `cwd`, `limit` and `headless` policy. The limit is `SESSION_LIST_LIMIT = 30` (`effects/mod.rs:61`). This is why the Grok picker shows one cwd group.
- External branch (`grok claude`, `external_agent == true`): sends a bare ACP `ListSessionsRequest` with no cwd filter, so the rows are whatever `claude-agent-acp` reports. Grok sessions are never requested here. The picker groups by cwd, which is why the Claude host shows several groups.

**Foreign scan.** `crates/codegen/xai-grok-pager/src/app/dispatch/session/foreign.rs:65` skips the `~/.claude` foreign scan when `chat_mode`, `external_agent` or Pi is set. So the Claude host does not merge in Grok rows and does not add foreign duplicates.

**Pi host.** `foreign.rs:82` sends `Effect::PiSessionList` for the Pi agent modal. `load.rs:1882` (`pi_catalog_host`) makes the Pi catalog the only external catalog. Commit `eca9236f` changed the Claude host from this catalog path to ACP `session/list` (`load.rs:1882-1883`, `load.rs:1950`).

**Loading.** Both hosts load through `dispatch_load_session` (`load.rs`) using ACP `session/load` for the active agent. The Claude host can only load sessions that `claude-agent-acp` lists. Load was not exercised in this scout.

**Plain `grok-fork` missing 13 rows.** `sessions list` on the CLI returns all 21 rows. The picker's `x.ai/session/list` path returns 8. I did not pin the exact line that drops the other 13. The 13 all have `chat_history.jsonl` of 2 lines or fewer. The first place to look is the headless policy applied in `foreign.rs:30-40` (`active_picker_headless_policy`) and the empty-row handling in `crates/codegen/xai-grok-shell/src/session/unified_list/mod.rs` (`lane_has_no_messages`, line 414). This is a hypothesis, not a verified cause.

## Smallest fix and files

No fix is needed to make plain `grok-fork` list Grok sessions: it already does.

Two decisions for Sab, not built here:

1. **If `grok claude` should also show Grok sessions:** the smallest change is in the external branch of `effects/mod.rs` (around line 797), which would also send `x.ai/session/list` for the cwd and merge the rows. The guard at `dispatch/session/foreign.rs:65` would need to change so the Claude host does not start double-listing Claude sessions. Files: `crates/codegen/xai-grok-pager/src/app/effects/mod.rs`, `crates/codegen/xai-grok-pager/src/app/dispatch/session/foreign.rs`.
2. **If the 13 hidden rows should appear in the plain picker:** confirm the cause in `unified_list/mod.rs` first (not yet pinned), then fix it there.

Neither change was tested. The load step was not exercised for any host.
