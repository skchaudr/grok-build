# Claude-session builtin audit (rule applied)

Rule: hide a builtin in Claude sessions when its action sends a grok-only `x.ai/*` request that stock claude-agent-acp does not implement. Keep it when it is pager-local, or when this branch already reroutes it for Claude.
Paths are under `crates/codegen/xai-grok-pager/src/`. Hidden names are in `HIDDEN` in `slash/claude_menu.rs`; a hidden name shows a notice when typed in full.
"Not traced" means I read only the `Action` and its router arm, not the whole code path.

## Hidden (this change)

| name | status | evidence |
|---|---|---|
| btw | hidden | `Effect::SendBtw` sends `x.ai/btw` (`app/effects/mod.rs:4649`) |
| session-info | hidden | `ShowSessionInfo`/`ShowContextInfo` both call `fetch_session_info` = `x.ai/session/info` (`app/effects/mod.rs:5421`, used at `:3765`, `:4000`); no Claude branch |
| share | hidden | `x.ai/share_session` (`app/effects/mod.rs:3684`) |
| delete | hidden | `x.ai/session/delete` (`app/effects/mod.rs:3887`) |
| skills | hidden | Skills tab fetch is `SKILLS_LIST_METHOD` = `x.ai/skills/list` (`app/effects/mod.rs:3123-3140`) |
| mcps | hidden | `x.ai/mcp/list` (`app/effects/mod.rs:2564`) |
| workflows | hidden | `x.ai/workflows/list` (`app/effects/mod.rs:3171`) |
| workflow | hidden | only `PassThrough("/workflow ...")` to a shell that has it (`slash/commands/workflow.rs:435-437`); the adapter has no `workflow` command |
| memory (mem) | hidden | `x.ai/memory/list` (`app/effects/mod.rs:2809-2816`); already yielded/gated by name |
| flush, dream | hidden | `x.ai/memory/{flush,dream}` (`app/effects/helpers.rs:375`); already shell-gated |
| imagine, imagine-video | hidden | `InjectSkill` prompt that tells the model to call Grok's image/video tools (`slash/commands/imagine.rs:3,29-36`, `imagine_video.rs:3,30`); Claude has no such tool |
| auto | hidden | auto-mode classifier lives in the Grok shell and is gated on `auto_mode_gate` (`app/acp_handler/settings.rs:383`); already fail-closed in `CommandRegistry::new` |

## Hidden (earlier hand-picked list, rechecked against the rule)

| name | status | evidence |
|---|---|---|
| login, logout | hidden | `x.ai/auth/*` (`app/effects/helpers.rs:852`, `app/effects/mod.rs:2489`); the adapter's own `/login` stays shown |
| privacy | hidden | `x.ai/privacy/setCodingDataRetention` (`app/effects/mod.rs:3940`) |
| feedback | hidden | `x.ai/feedback` (`app/effects/mod.rs:4123`) |
| marketplace | hidden, typed text forwarded | `x.ai/marketplace/list` (`app/effects/mod.rs:3048`); forwarded by `claude_forwards_builtin` |
| personas, config-agents | hidden | bundle/agents modal (`x.ai/bundle/status`, `app/effects/mod.rs:4735`); `config-agents` alias `agents` is released to the adapter |
| recap | hidden, adapter's `/recap` shown when advertised | `x.ai/recap` (`app/effects/mod.rs:4687`); typed `/summarize` goes to the adapter's `/recap` |
| loop | hidden (not by the x.ai rule) | `InjectSkill`, tool-gated off for Claude, so it never ran; kept in `HIDDEN` only so the adapter's own `/loop` can claim the name |

## Un-hidden (pager-local, no `x.ai/*` request)

| name | status | evidence |
|---|---|---|
| announcements | visible | `Action::AnnouncementsHide/Show`; self-gates on `has_session_announcements` |
| tutorial, docs, release-notes | visible | `OpenTutorial`, `OpenHowtoGuides`/`OpenUrl`, `ShowReleaseNotes` |
| import-claude | visible | `Action::ImportClaudeSettings`, local settings import |
| voice | visible | `Action::VoiceToggle`; still fail-closed in `CommandRegistry::new` |

Typed in full, any `HIDDEN` builtin shows "/<name> isn't available in Claude sessions" (`typed_builtin_override`, called from `app/dispatch/prompt.rs`) instead of sending its request.

## Kept

| name | status | evidence |
|---|---|---|
| compact, usage, context | kept (adapter) | `YIELDED_TO_EXTERNAL_AGENT` (`slash/registry.rs:~119`); the adapter owns the name |
| model | kept | `SwitchModel`, standard ACP model config |
| context-window | kept | `SwitchModel` (`slash/commands/context_window.rs`); errors if the model has no variants. Not traced |
| effort, hooks, plugins (plugin), marketplace | kept, forwarded as typed text | `CLAUDE_FORWARDED_BUILTINS` (`slash/external_common.rs`) |
| rename | kept | rerouted for Claude: `claude_title::append_custom_title` (`app/effects/mod.rs:3808`) |
| rewind | kept | rerouted: `Effect::ClaudeRewindFork` (`app/effects/mod.rs:4892`) |
| fork | kept | rerouted: standard `session/fork` for Claude (`app/effects/mod.rs:5019-5027`) |
| resume | kept | rerouted per the earlier commits (resume search by cwd); not re-traced |
| new, exit (quit), copy, export | kept | pager-local (`Action::NewSession`, `Quit` `router.rs:168`, `CopyAssistantMessage` `router.rs:709`, `ExportConversation` `router.rs:713`) |
| settings | kept | `OpenSettings` (`app/dispatch/router.rs:1225`); local settings modal. Not traced |
| plan | kept | `SetSessionMode("plan")` = standard `session/set_mode` (`app/dispatch/modes.rs:101-106`, `app/effects/mod.rs:1839`) |
| view-plan | kept | pager-local (`app/dispatch/modes.rs:15`) |
| queue | kept | prints a local block (`app/dispatch/status.rs:469`) |
| tasks | kept | prints a local block (`app/dispatch/status.rs:482`) |
| history | kept | local history search (`app/dispatch/prompt.rs:512`) |
| transcript (log) | kept | `OpenTranscriptPager`, local (`app/dispatch/router.rs:717`) |
| find | kept | `OpenScrollbackSearch`, local (`app/dispatch/router.rs:545`) |
| jump, expand, edit-prompt | kept | local (`router.rs:1698`, `router.rs:721`; `EditPromptExternal`) |
| always-approve | kept | the pager auto-approves permission requests itself (`app/acp_handler/permissions.rs:25`); the `x.ai/yolo_mode_changed` notification (`app/effects/helpers.rs:1499`) is ignored by the adapter |
| theme, vim-mode, multiline, compact-mode, timestamps, timeline, toggle-mouse-reporting, minimal/fullscreen, help, home, gboom, debug | kept | pager-local actions (router arms `:1211`, `:1163`, `:1190`, `:1162`, `:1193`, `:1194`, `:659`, `:193`, `:1229`, `:278`). Not traced past the arm |
| traces, dashboard, cd | kept | pager views (`OpenTraces` `router.rs:1447`, `OpenDashboard` `router.rs:1550`). Dashboard is hidden until its gate is set. Not traced |
| doctor | kept | local report from pager state (`app/dispatch/prompt.rs:97-125`); `PlanDoctorFix` not traced |
| remember | kept by the rule, doubtful in use | saves to Grok's local memory store (`app/effects/mod.rs:4560-4573`), which Claude never reads; no `x.ai` request, so the rule keeps it. Hide it if you want |
