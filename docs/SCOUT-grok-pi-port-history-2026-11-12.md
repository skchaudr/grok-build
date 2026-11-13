# Scout: grok-pi port history (Opus sessions), 2026-11-12

Read-only. Sources: Claude Code transcripts under `~/.claude/projects/` (Mini), git on `main` and all refs, `git worktree list`, and sab-air's `~/.grok/bin`.

## Bottom line

- **Landed on main:** steps 1 to 5 of `docs/grok-pi-dual-agent-map.md` (commits `6246c4f3` to `d3a9bacd`), plus the picker and launcher work (`1ead2f71`, `2d60e6d6`, `ab8cfd6d`). Pi runs inside the fork pager through `pig`, `gb grok-pi` and `gb claude grok-pi`.
- **Never landed:** step 6 (Pi settings `/pi-config`, `/pi-models`, `/pi-shortcut-manager`, `/eval-display`, `/scoped-models`; the 100+ Pi themes; the dpi shim). Also never landed: grok-pi's logo, welcome brand and tour. `git grep PI_LOGO origin/main` and `ExternalWelcomeBrand` return nothing.
- **No unmerged branches or worktrees hold port work.** `pi/import-adapter` was never created. `gc/claude-parity` was merged and deleted. `grok-build-pi-wip` was removed on 11-10.
- **Why Sab sees none of it:** the visible grok-pi he runs is Dwsy's standalone app (`~/.local/bin/grok-pi`). The ported surface is `pig`, which has no logo, welcome, settings or themes. Its basic arrow keys were also broken until `5afdbfab`. The reports said "steps 1-5 done", which reads as "ported", and step 6 was parked "until you call it".

## Timeline

1. **2026-10-26, `f6c9da3d`** (docs/traces/SPEC-usable.md, Track B). Sab's intent: "of course I want a working Pi in the GROK TUI with all the features it comes with." Not a session; context only.

2. **2026-11-08 to 11-09, Opus, `~/.claude/projects/-Volumes-Mini-SSD-offload-repos-grok-build/9643f8cd-6201-4cf9-8eec-96e49917f4f4.jsonl`** (plan session, 22:37 to 01:31).
   - Asked: a second opinion on grok-pi and grok-fork conflicts. Sab wanted routing so Claude and Pi commands go to whichever agent is live.
   - Claimed: a plan with no model spend. Proposed slices `pi/import-adapter`, `pi/stdio-bin`, `pi/per-session-agent`.
   - Landed: `gc/claude-parity` was folded into main (`357dc39e`) and its branch deleted. The map itself is on main as `7b65e68e` and `f1b55b07`.
   - Not landed: no `pi/*` branch exists locally or on origin. The session ends on "Go on that slice?" with no reply. No build ran.

3. **2026-11-07 to 11-12, Opus, `~/.claude/projects/-Volumes-Mini-SSD-offload-repos-grok-build/40e362e8-0d31-4d5a-bd05-916ffa9f77a5.jsonl`** ("Grok Claude parity gaps", 47 MB). This is the planner and brief-writer for the port.
   - Asked: Claude parity in the fork. Sab's words on 11-09: "I meant for grok pi to be one harness grokbuild our fork chooses from."
   - Landed on main: the gc/claude-parity pushes (11-09 00:56 and 01:27), traces merges, `3ac4aae7` (grok-pi harness brief, 11-09 17:26), and `3af1e574` (brief amended to "run steps 1-5, stop before step 6", 21:10).
   - At 11-08 22:23 it recorded Sab saying he did not see Pi being folded in "sooner than this panel can be completed." The traces side panel took priority from then on.

4. **2026-11-09 17:26 to 11-11 07:53, Opus, `~/.claude/projects/-Volumes-Mini-SSD-offload-repos-grok-build/72f9e4cb-7725-41a2-af90-6ab8748266a7.jsonl`** (the executor, 28 MB). This is the main port session.
   - Asked: "Read docs/traces/briefs/grok-pi-harness.md and do exactly what it says." Scope from the brief: steps 1 to 5, then stop before step 6.
   - Claimed and verified on main:
     - Step 1 `6246c4f3` (adapter, 23 extensions, `grok-pi-acp`). Step 2 `2003e4e6` (per-session agent profile). Step 3 `18f46cb3` (pi/ui notifications).
     - Step 3 `33b59957` (Remote TUI, shortcuts, /resume), `c76b9535` (two leader bugs found on screen).
     - Step 4 `2eafa81f` (Codemode and Eval cards; both renderers kept).
     - Step 5 `d3a9bacd` (Pi slash commands).
     - Picker and launcher: `1ead2f71` (grok-pi in gb), `2d60e6d6` (one leader per gb launch, pushed 11-10 12:56), `ab8cfd6d` (`pig` runs the fork's grok-pi, 11-10 15:06).
     - Progress docs `dcb20481`, `9a592133`.
   - Side checkout: `/Volumes/Mini-SSD/offload/repos/grok-build-pi-wip` (detached, 11-10 05:24), reported removed. It is not in `git worktree list`.
   - Installs: `grok-fork` and `grok-pi-acp` rsynced to sab-air at 11-10 12:34 after Sab's "Yes". On sab-air now: `~/.grok/bin/grok-fork` dated 11-12 15:33, `grok-pi-acp` dated 11-09 22:59.
   - Claimed and kept: "grok-pi now works on both machines" (11-11 07:53, checked on screen). Its own last line: "Pi's own settings in grok-pi is the next step, and it waits for you."
   - Not landed: step 6 and the branding. The brief excluded themes from this run.

5. **2026-11-12 to 11-13, Opus lead, `~/.claude/projects/-Volumes-Mini-SSD-offload-repos-grok-build/2cb78e6b-d2f5-4c74-8c68-9c501f4d1847.jsonl`** (Herdr Grok traces TUI audit, not a port session).
   - 11-12 21:26: Sab said `pig` "which I thought was grok-pi is deeply not working." The lead confirmed `pig` runs the fork's grok-pi, and `grok-pi` is Dwsy's original.
   - 11-12 21:48: root cause for the broken basics: the pager ignores Shift on arrow keys, so plain Up/Down match Pi's `shift+up`/`shift+down` workflow shortcuts. Fixed in `5afdbfab`, tests pass.
   - 11-13 01:40: the lead's history answer. Steps 1 to 5 landed, the brief stopped at step 6, and Sab's "all the features it comes with" was marked optional by an agent.
   - Its subagent `agent-a782e49271cc1def9.jsonl` is the scout that wrote this report.
   - The sidebar name "explore-pi-extensions" matches only this session and its subagents. No separate transcript exists under that name.

6. **Checked and excluded:** `~/.claude/projects/-Users-sab-mini-repos-pi-durable-lab/2024ed79-2c54-44e1-8fcd-e00417a42980.jsonl` (sidebar "Opus 5.5 - pi-durable-lab"). It is the DPI demo and has no grok-pi port work. `~/.claude/projects/-Users-sab-mini/445ca55f-5da2-477f-9cf7-ac18a8116bb7.jsonl` is a delta-brief script session with no port.

## Git state

| Item | Where | State |
|---|---|---|
| `6246c4f3` ... `d3a9bacd`, `2d60e6d6`, `ab8cfd6d`, `c76b9535`, `1ead2f71` | main | on origin/main |
| `04dda258`, `5731ca69` | hub-live only | not on main (the hub copy of gb and pig) |
| `b8f926f1` | main, 11-12 | adds reference screenshots `docs/images/grok-pi-logo-welcome.png` and `grok-pi-distinct-settings-modal.png` (targets for step 6) |
| `5afdbfab` | main, 11-12 | arrow fix from the lead session |
| `pi/import-adapter`, `pi/*` | nowhere | never created |
| `gc/claude-parity` | nowhere | merged to main and deleted |
| `grok-build-pi-wip`, `grok-build-gc-parity` | nowhere | removed |
| `/Users/sab-mini/repos/grok-build` | second clone | on main at `b60ee0ec`, clean except `.worktrees/` |
| `/Volumes/Mini-SSD/offload/repos/grok-pi` | path from the brief | does not exist on the Mini |
| sab-air `~/api/grok-build` | path from the brief | not a git repo there |

## Why Sab sees none of it

1. **Wrong binary in his head.** `grok-pi` on the Mini is Dwsy's standalone app, still installed. The port is reached only through `pig` or `gb grok-pi`, not through `grok` or `grok pick`.
2. **Step 6 was never done, and the port was labelled as the whole job.** Logo, welcome tour, tabbed settings, theme picker and the dpi shim were all in step 6 or outside the brief. Reports said "steps 1-5 done", and the "waits for you" line sat at the end of a long session.
3. **Basic keys broke the surface.** Plain Up/Down were captured by Pi's shift shortcuts (`33b59957` added the Pi shortcut path, `5afdbfab` fixed it on 11-12). Nobody checked it on screen before the step-6 pause.
4. **Attention went elsewhere.** From 11-08 22:23 Sab's focus moved to the traces side panel and follow view, and the step-6 hand-off got buried in that work.

## Open for Sab

- Confirm whether step 6 (settings, themes, logo, welcome, tour) is the next port. Nothing in git says it is scheduled.
- Decide whether `grok-pi` should mean Dwsy's standalone app or the fork's `pig`.
