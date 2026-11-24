import fs from "node:fs";
import path from "node:path";
import {
  agent,
  choice,
  compute,
  defineHumanChoices,
  defineWorkflow,
  humanDecision,
  humanDecisionEdge,
  idempotentEffect,
  shell,
} from "@osolmaz/pi-workflows";

const AIR_WORKTREE = "/Users/sab-mini/repos/grok-build/.worktrees/beta8-review";
const MINI_ROOT = "/Volumes/Mini-SSD/offload/repos/grok-build/.worktrees";
const MINI_SHELL_WT = `${MINI_ROOT}/beta8-shell`;
const MINI_PAGER_WT = `${MINI_ROOT}/beta8-pager`;
const MINI_REST_WT = `${MINI_ROOT}/beta8-rest`;

const GROUP1_AIR_PATHS = [
  ".gitignore",
  "AGENTS.md",
  "CHANGELOG.MD",
  "Cargo.lock",
  "Cargo.toml",
  "crates/codegen/xai-grok-agent/src/builder.rs",
  "crates/codegen/xai-grok-announcements/Cargo.toml",
  "crates/codegen/xai-grok-config-types/Cargo.toml",
  "crates/codegen/xai-grok-config-types/src/mcp.rs",
  "crates/codegen/xai-grok-config/src/lib.rs",
  "crates/codegen/xai-grok-login/src/auth_method.rs",
  "crates/codegen/xai-grok-login/src/config.rs",
  "crates/codegen/xai-grok-login/src/credential_provider.rs",
  "crates/codegen/xai-grok-login/src/manager.rs",
  "crates/codegen/xai-grok-login/src/meta.rs",
  "crates/codegen/xai-grok-mcp/src/oauth_config.rs",
  "crates/codegen/xai-grok-mcp/src/servers.rs",
  "crates/codegen/xai-grok-memory/src/v2_access_tests.rs",
  "crates/codegen/xai-grok-pager-bin/Cargo.toml",
  "crates/codegen/xai-grok-pager-bin/src/bin/grok-pi.rs",
  "crates/codegen/xai-grok-pager-bin/src/main.rs",
  "crates/codegen/xai-grok-pager-minimal/Cargo.toml",
  "crates/codegen/xai-grok-pager-render/src/prompt_images.rs",
  "crates/codegen/xai-grok-sampler/src/config.rs",
  "crates/codegen/xai-grok-sampler/src/metrics.rs",
  "crates/codegen/xai-grok-sampling-types/src/lib.rs",
  "crates/codegen/xai-grok-shared/src/lib.rs",
  "crates/codegen/xai-grok-shell-base/src/lib.rs",
  "crates/codegen/xai-grok-shell-terminal/src/background_task.rs",
  "crates/codegen/xai-grok-tools/src/implementations/opencode/skill/mod.rs",
  "crates/codegen/xai-grok-tools/src/types/api_key_provider.rs",
  "crates/codegen/xai-grok-tools/src/util/truncate.rs",
  "crates/codegen/xai-grok-workspace/Cargo.toml",
  "crates/codegen/xai-grok-workspace/src/config.rs",
  "crates/codegen/xai-grok-workspace/src/file_system/mod.rs",
  "crates/codegen/xai-grok-workspace/src/fs_notify.rs",
  "crates/codegen/xai-grok-workspace/src/handle.rs",
  "crates/codegen/xai-grok-workspace/src/hub.rs",
  "crates/codegen/xai-grok-workspace/src/mcp.rs",
  "crates/codegen/xai-grok-workspace/src/permission/manager/mod.rs",
  "crates/codegen/xai-grok-workspace/src/permission/types.rs",
  "crates/common/xai-tool-types/src/lib.rs",
  "docs/CHANGELOG.zh-CN.md",
  "rust-toolchain.toml",
] as const;

const GROUP1_EXTRA_WIRING = [
  "crates/codegen/xai-grok-pager-bin/src/bin/grok-pi-acp.rs",
] as const;

const GROUP2_SHELL_PATHS = [
  "crates/codegen/xai-grok-shell/Cargo.toml",
  "crates/codegen/xai-grok-shell/src/agent/config.rs",
  "crates/codegen/xai-grok-shell/src/agent/mod.rs",
  "crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs",
  "crates/codegen/xai-grok-shell/src/agent/subagent/handle_request.rs",
  "crates/codegen/xai-grok-shell/src/agent/subagent/spawn.rs",
  "crates/codegen/xai-grok-shell/src/claude_import.rs",
  "crates/codegen/xai-grok-shell/src/config/mod.rs",
  "crates/codegen/xai-grok-shell/src/extensions/mod.rs",
  "crates/codegen/xai-grok-shell/src/leader/mod.rs",
  "crates/codegen/xai-grok-shell/src/lib.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_impl/hooks_plugins.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_impl/model_switch.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_impl/run_loop.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_impl/spawn.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_impl/tool_calls.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_impl/turn.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_tests/cancel_running_task_tests.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_tests/image_strip_tests.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_tests/slash_authority_turn_tests.rs",
  "crates/codegen/xai-grok-shell/src/session/acp_session_tests/transient_retry_tests.rs",
  "crates/codegen/xai-grok-shell/src/session/commands.rs",
  "crates/codegen/xai-grok-shell/src/session/compaction_inline_auto_compact_flow_tests.rs",
  "crates/codegen/xai-grok-shell/src/session/helpers/session_compact.rs",
  "crates/codegen/xai-grok-shell/src/session/image_describe.rs",
  "crates/codegen/xai-grok-shell/src/session/managed_mcp.rs",
  "crates/codegen/xai-grok-shell/src/session/mod.rs",
  "crates/codegen/xai-grok-shell/src/session/workflow/host_service.rs",
  "crates/codegen/xai-grok-shell/src/session/workflow/manager.rs",
  "crates/codegen/xai-grok-shell/src/test_support/lsp_runtime.rs",
  "crates/codegen/xai-grok-shell/src/upload/manifest.rs",
  "crates/codegen/xai-grok-shell/src/upload/mod.rs",
  "crates/codegen/xai-grok-shell/src/util/config/mcp.rs",
  "crates/codegen/xai-grok-shell/src/util/config/persist.rs",
] as const;

const GROUP3_PAGER_PATHS = [
  "crates/codegen/xai-grok-pager/Cargo.toml",
  "crates/codegen/xai-grok-pager/src/app/actions.rs",
  "crates/codegen/xai-grok-pager/src/app/agent_view/fork_picker.rs",
  "crates/codegen/xai-grok-pager/src/app/agent_view/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/agent_view/prompt.rs",
  "crates/codegen/xai-grok-pager/src/app/agent_view/render.rs",
  "crates/codegen/xai-grok-pager/src/app/agent_view/selection.rs",
  "crates/codegen/xai-grok-pager/src/app/agent_view/session.rs",
  "crates/codegen/xai-grok-pager/src/app/app_view.rs",
  "crates/codegen/xai-grok-pager/src/app/app_view_tests.rs",
  "crates/codegen/xai-grok-pager/src/headless.rs",
  "crates/codegen/xai-grok-pager/src/lib.rs",
  "crates/codegen/xai-grok-pager/src/notifications/config.rs",
  "crates/codegen/xai-grok-pager/src/notifications/protocol.rs",
  "crates/codegen/xai-grok-pager/src/scrollback/blocks/tool/other.rs",
  "crates/codegen/xai-grok-pager/src/scrollback/render_tests.rs",
  "crates/codegen/xai-grok-pager/src/scrollback/state/groups.rs",
  "crates/codegen/xai-grok-pager/src/scrollback/state/mod.rs",
  "crates/codegen/xai-grok-pager/src/scrollback/state/selection_tests.rs",
  "crates/codegen/xai-grok-pager/src/scrollback/state/verb_group.rs",
  "crates/codegen/xai-grok-pager/src/settings/layout.rs",
  "crates/codegen/xai-grok-pager/src/settings/mod.rs",
  "crates/codegen/xai-grok-pager/src/settings/registry.rs",
  "crates/codegen/xai-grok-pager/src/slash/command.rs",
  "crates/codegen/xai-grok-pager/src/slash/commands/model.rs",
  "crates/codegen/xai-grok-pager/src/slash/mod.rs",
  "crates/codegen/xai-grok-pager/src/trace_cmd.rs",
  "crates/codegen/xai-grok-pager/src/views/agent.rs",
  "crates/codegen/xai-grok-pager/src/views/agents_modal.rs",
  "crates/codegen/xai-grok-pager/src/views/block_viewer/block_viewer_tests.rs",
  "crates/codegen/xai-grok-pager/src/views/file_search/dropdown.rs",
  "crates/codegen/xai-grok-pager/src/views/file_search/state.rs",
  "crates/codegen/xai-grok-pager/src/views/settings_modal/state.rs",
  "crates/codegen/xai-grok-pager/src/views/settings_modal/tests.rs",
  "crates/codegen/xai-grok-pager/src/views/timeline.rs",
  "crates/codegen/xai-grok-pager/src/views/turn_status.rs",
  "crates/codegen/xai-grok-pager/src/voice/mod.rs",
  "crates/codegen/xai-grok-pager/tests/settings_e2e.rs",
] as const;

const GROUP4_KEEP_PATHS = [
  "crates/codegen/xai-grok-pager/src/acp/mod.rs",
  "crates/codegen/xai-grok-pager/src/acp/model_state.rs",
  "crates/codegen/xai-grok-pager/src/acp/tracker.rs",
  "crates/codegen/xai-grok-pager/src/acp/tracker_tests.rs",
  "crates/codegen/xai-grok-pager/src/app/acp_handler/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/acp_handler/tests/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/acp_handler/tests/settings.rs",
  "crates/codegen/xai-grok-pager/src/app/acp_handler/tests/subagents.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/ctx.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/dashboard.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/router.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/session/fork.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/session/load.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/session/modal.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/settings/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/settings/setters.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/task_result.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/dashboard.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/notes.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/router.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/session/fork.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/session/lifecycle.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/session/load.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/settings.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/tests/task_result.rs",
  "crates/codegen/xai-grok-pager/src/app/effects/mod.rs",
  "crates/codegen/xai-grok-pager/src/app/effects/tests.rs",
  "crates/codegen/xai-grok-pager/src/app/event_loop.rs",
  "crates/codegen/xai-grok-pager/src/app/mermaid_worker.rs",
  "crates/codegen/xai-grok-pager/src/app/mod.rs",
] as const;

const GROUP4_DELETE_PATHS = [
  "crates/codegen/xai-grok-pager/src/app/dispatch/session/pi_fork.rs",
  "crates/codegen/xai-grok-pager/src/app/dispatch/session/tree.rs",
] as const;

const GROUP4_SESSION_TARGET = "crates/codegen/xai-grok-pager/src/app/dispatch/session/pi_session.rs";

type ResolveOutput = {
  group: number;
  resolvedCount: number;
  notes: string;
};

function assertNoConflictMarkers(
  worktree: string,
  keepPaths: readonly string[],
  deletePaths: readonly string[] = [],
): void {
  const stillConflicted: string[] = [];
  const missing: string[] = [];
  const markerRe = /^(<<<<<<<|\|\|\|\|\|\|\||=======|>>>>>>>)(\s|$)/m;

  for (const rel of keepPaths) {
    const full = path.join(worktree, rel);
    if (!fs.existsSync(full)) {
      missing.push(rel);
      continue;
    }
    const text = fs.readFileSync(full, "utf8");
    if (markerRe.test(text)) {
      stillConflicted.push(rel);
    }
  }
  const notDeleted = deletePaths.filter((rel) => fs.existsSync(path.join(worktree, rel)));
  if (missing.length > 0) {
    throw new Error(`Missing expected files in ${worktree}: ${missing.join(", ")}`);
  }
  if (stillConflicted.length > 0) {
    throw new Error(
      `${stillConflicted.length} file(s) in ${worktree} still contain conflict markers: ${stillConflicted.join(", ")}`,
    );
  }
  if (notDeleted.length > 0) {
    throw new Error(
      `Modify/delete file(s) in ${worktree} must be removed after porting their behavior: ${notDeleted.join(", ")}`,
    );
  }
}

function validateResolveOutput(
  group: number,
  worktree: string,
  keepPaths: readonly string[],
  deletePaths: readonly string[] = [],
) {
  return (raw: unknown): ResolveOutput => {
    const out = raw as Partial<ResolveOutput> | null;
    if (!out || typeof out.notes !== "string" || out.notes.trim().length === 0) {
      throw new Error("Submission must include non-empty notes summarizing the resolution.");
    }
    assertNoConflictMarkers(worktree, keepPaths, deletePaths);
    return {
      group,
      resolvedCount: keepPaths.length + deletePaths.length,
      notes: out.notes.trim(),
    };
  };
}

const airGateChoices = defineHumanChoices({
  gather: choice({ label: "Air Group 1 is done — gather all 151 paths and run cargo check" }),
  stop: choice({ label: "Stop before gathering" }),
});

const reviewGateChoices = defineHumanChoices({
  done: choice({ label: "Accept merged tree" }),
  repair: choice({ label: "Run Claude Sonnet repair pass on compile errors" }),
});

export default defineWorkflow({
  name: "beta8-merge",
  title: "Merge Dwsy v0.1.14-beta.8 into Grok Build across 4 worktrees",
  startAt: "route_host",
  maxSteps: 30,
  nodes: {
    route_host: compute({
      run: ({ input }) => {
        const raw = (input ?? {}) as { task?: string; host?: string; mode?: string };
        // Exact values only: a substring test sent "repair ..." down the Air path.
        for (const value of [raw.host, raw.mode, raw.task]) {
          const v = (value ?? "").trim().toLowerCase();
          if (v === "air") return { host: "air" as const };
          if (v === "mini") return { host: "mini" as const };
        }
        const isMini = fs.existsSync(MINI_SHELL_WT);
        return { host: isMini ? ("mini" as const) : ("air" as const) };
      },
    }),

    // Group 1 on sab-air (beta8-review): Pi with GLM 5.3 (Air's default Pi model)
    resolve_group1_air: agent({
      timeoutMs: 3_600_000,
      prompt: () =>
        [
          `Work strictly inside ${AIR_WORKTREE}.`,
          "You are resolving Group 1 of the Grok Build + Dwsy v0.1.14-beta.8 merge (44 files outside xai-grok-pager and xai-grok-shell, plus wiring the mentions extension in grok-pi-acp.rs).",
          "Five files in this group were already resolved in this worktree (AGENTS.md, CHANGELOG.MD, crates/codegen/xai-grok-agent/src/builder.rs, crates/codegen/xai-grok-login/src/credential_provider.rs, crates/codegen/xai-grok-login/src/manager.rs). Verify them and resolve the remaining 39 files.",
          "Resolution rules:",
          "1. Keep both sides' intent: preserve our fork's multi-host, traces, acpx, piw, and isolated grok-home wiring while adopting upstream v0.1.14-beta.8 additions.",
          "2. In crates/codegen/xai-grok-pager-bin/src/bin/grok-pi-acp.rs (and grok-pi.rs / main.rs), ensure the new mentions_extension (and emphasis / working_message extensions if added to grok-pi.rs) is loaded by grok-pi-acp so @ object mentions work in external Pi ACP sessions.",
          "3. In Cargo.toml, Cargo.lock, and rust-toolchain.toml, reconcile workspace dependencies so both the fork crates (xai-grok-traces, grok-acpx-acp, grok-pi-acp) and upstream v0.1.14-beta.8 crates resolve.",
          "4. Stage each resolved Group 1 file with `git add <file>`. Do NOT touch any file under crates/codegen/xai-grok-pager/ or crates/codegen/xai-grok-shell/, and do NOT run `git commit`.",
          "",
          "Group 1 files:",
          ...GROUP1_AIR_PATHS.map((p) => `- ${p}`),
          ...GROUP1_EXTRA_WIRING.map((p) => `- ${p} (wire mentions_extension)`),
          "",
          'When all 44 conflicted files and grok-pi-acp.rs have zero conflict markers, call the workflow tool to submit {"group": 1, "resolvedCount": 45, "notes": "<concise summary>"}.',
        ].join("\n"),
      expectedOutput: '{"group": 1, "resolvedCount": 45, "notes": "summary of Group 1 resolution"}',
      validate: validateResolveOutput(1, AIR_WORKTREE, [
        ...GROUP1_AIR_PATHS,
        ...GROUP1_EXTRA_WIRING,
      ]),
    }),

    air_done: compute({
      run: ({ outputs }) => ({
        host: "air",
        group1: outputs.resolve_group1_air,
      }),
    }),

    // Group 2 on Mini (beta8-shell): Grok 4.7 via ACPX
    resolve_group2_shell: agent({
      agent: "grok-build",
      model: "grok-4.7",
      timeoutMs: 3_600_000,
      prompt: () =>
        [
          `Work strictly inside ${MINI_SHELL_WT}.`,
          "You are resolving Group 2 of the Grok Build + Dwsy v0.1.14-beta.8 merge: all 35 conflicted files in crates/codegen/xai-grok-shell/.",
          "Resolution rules:",
          "1. Resolve every conflict marker (<<<<<<<, =======, >>>>>>>) in the 35 xai-grok-shell files listed below, keeping both our fork's behavior and upstream v0.1.14-beta.8's additions.",
          "2. Stage each resolved file with `git add <file>`. Do NOT touch any file outside crates/codegen/xai-grok-shell/ and do NOT run `git commit`.",
          "",
          "Group 2 files (35):",
          ...GROUP2_SHELL_PATHS.map((p) => `- ${p}`),
          "",
          'When all 35 files have zero conflict markers, call the workflow tool to submit {"group": 2, "resolvedCount": 35, "notes": "<concise summary>"}.',
        ].join("\n"),
      expectedOutput: '{"group": 2, "resolvedCount": 35, "notes": "summary of Group 2 shell resolution"}',
      validate: validateResolveOutput(2, MINI_SHELL_WT, GROUP2_SHELL_PATHS),
    }),

    // Group 3 on Mini (beta8-pager): Pi on Mini
    resolve_group3_pager: agent({
      timeoutMs: 3_600_000,
      prompt: () =>
        [
          `Work strictly inside ${MINI_PAGER_WT}.`,
          "You are resolving Group 3 of the Grok Build + Dwsy v0.1.14-beta.8 merge: 38 conflicted files in crates/codegen/xai-grok-pager/ covering Cargo.toml, agent_view, app_view, notifications, scrollback, settings, slash commands, views, and voice.",
          "Resolution rules:",
          "1. Keep both sides' intent: preserve our fork's acpx modal/flow blocks, piw integration, traces view, side panel, and theme work while adopting upstream v0.1.14-beta.8 features (@ mention chips, thinking_stream_mode, tool-call density, emphasis pills, timeline config).",
          "2. Stage each resolved file with `git add <file>`. Do NOT touch files outside this 38-file list and do NOT run `git commit`.",
          "",
          "Group 3 files (38):",
          ...GROUP3_PAGER_PATHS.map((p) => `- ${p}`),
          "",
          'When all 38 files have zero conflict markers, call the workflow tool to submit {"group": 3, "resolvedCount": 38, "notes": "<concise summary>"}.',
        ].join("\n"),
      expectedOutput: '{"group": 3, "resolvedCount": 38, "notes": "summary of Group 3 pager UI resolution"}',
      validate: validateResolveOutput(3, MINI_PAGER_WT, GROUP3_PAGER_PATHS),
    }),

    // Group 4 on Mini (beta8-rest): Claude Sonnet 5.5 via ACPX
    resolve_group4_acp_session: agent({
      agent: "claude",
      model: "sonnet",
      timeoutMs: 3_600_000,
      prompt: () =>
        [
          `Work strictly inside ${MINI_REST_WT}.`,
          "You are resolving Group 4 of the Grok Build + Dwsy v0.1.14-beta.8 merge: 34 conflicted files in crates/codegen/xai-grok-pager/src/{acp,app}/ (ACP tracker, ACP handler, dispatch router, effects, event loop, and Pi session navigation).",
          "Critical architecture rules from the dry-run report:",
          "1. Pager & ACP routing (F5): our fork routes Pi and Claude notifications through its own handlers. Beta.8 adds mention catalogs, working messages, and timeline updates at that same boundary. Keep our fork's routing and add the upstream messages to it.",
          `2. Modify/delete conflicts in Pi session navigation (F6): our fork moved functionality from pi_fork.rs and tree.rs into ${GROUP4_SESSION_TARGET}. Beta.8 modified those old files to add fork-to-dashboard and faster tree replay. Port those new beta.8 behaviors into ${GROUP4_SESSION_TARGET}, then remove the two old files with \`git rm -f ${GROUP4_DELETE_PATHS.join(" ")}\` so no second routing path is restored.`,
          "3. Stage your resolved files with `git add`. Do NOT touch files outside Group 4 and do NOT run `git commit`.",
          "",
          "Group 4 files to resolve and keep (32):",
          ...GROUP4_KEEP_PATHS.map((p) => `- ${p}`),
          "",
          "Group 4 modify/delete files to port into pi_session.rs and git rm (2):",
          ...GROUP4_DELETE_PATHS.map((p) => `- ${p} -> port into ${GROUP4_SESSION_TARGET} and remove`),
          "",
          'When all 32 kept files and pi_session.rs have zero conflict markers and the 2 deleted files are removed, call the workflow tool to submit {"group": 4, "resolvedCount": 34, "notes": "<concise summary>"}.',
        ].join("\n"),
      expectedOutput: '{"group": 4, "resolvedCount": 34, "notes": "summary of Group 4 ACP and session resolution"}',
      validate: validateResolveOutput(
        4,
        MINI_REST_WT,
        [...GROUP4_KEEP_PATHS, GROUP4_SESSION_TARGET],
        GROUP4_DELETE_PATHS,
      ),
    }),

    wait_for_air_gate: humanDecision({
      audience: "operator",
      choices: airGateChoices,
      request: ({ outputs }) => ({
        title: "Mini Groups 2, 3, and 4 are resolved. Is Air Group 1 finished?",
        subject: {
          group2: outputs.resolve_group2_shell,
          group3: outputs.resolve_group3_pager,
          group4: outputs.resolve_group4_acp_session,
        },
        presentation: {
          schema: "pi-workflows.decision-presentation.v1",
          summary:
            "Confirm that the Air run in beta8-review finished resolving Group 1. Continuing will copy only the explicit Group 1, 2, and 3 file lists into beta8-rest and run cargo check.",
          blocks: [],
        },
      }),
    }),

    gather_resolved_files: shell({
      effect: idempotentEffect("beta8.gather_resolved_files"),
      timeoutMs: 300_000,
      exec: () => {
        const g1List = [...GROUP1_AIR_PATHS, ...GROUP1_EXTRA_WIRING];
        const g2List = [...GROUP2_SHELL_PATHS];
        const g3List = [...GROUP3_PAGER_PATHS];
        const g4Keep = [...GROUP4_KEEP_PATHS, GROUP4_SESSION_TARGET];
        const g4Del = [...GROUP4_DELETE_PATHS];

        const script = [
          "set -euo pipefail",
          `cd "${MINI_REST_WT}"`,
          "echo '1. Pulling Group 1 files from sab-air beta8-review...'",
          `ssh -o BatchMode=yes sab-mini@sab-air "tar -C '${AIR_WORKTREE}' -cf - ${g1List.map((p) => `'${p}'`).join(" ")}" | tar -C "${MINI_REST_WT}" -xf -`,
          "echo '2. Copying Group 2 files from Mini beta8-shell...'",
          `tar -C "${MINI_SHELL_WT}" -cf - ${g2List.map((p) => `'${p}'`).join(" ")} | tar -C "${MINI_REST_WT}" -xf -`,
          "echo '3. Copying Group 3 files from Mini beta8-pager...'",
          `tar -C "${MINI_PAGER_WT}" -cf - ${g3List.map((p) => `'${p}'`).join(" ")} | tar -C "${MINI_REST_WT}" -xf -`,
          "echo '4. Checking gathered files for conflict markers...'",
          `if grep -lE '^(<<<<<<<|[|]{7}|=======|>>>>>>>)( |$)' -- ${[...g1List, ...g2List, ...g3List, ...g4Keep].map((p) => `'${p}'`).join(" ")}; then echo 'Refusing to stage: the files above still contain conflict markers'; exit 1; fi`,
          "echo '5. Staging all 151 paths in beta8-rest...'",
          `git rm -f --ignore-unmatch ${g4Del.map((p) => `'${p}'`).join(" ")}`,
          `git add -- ${[...g1List, ...g2List, ...g3List, ...g4Keep].map((p) => `'${p}'`).join(" ")}`,
          "UNMERGED=$(git diff --name-only --diff-filter=U | wc -l | tr -d ' ')",
          'echo "Remaining unmerged files: $UNMERGED"',
          'test "$UNMERGED" = "0"',
        ].join("\n");

        return {
          command: "bash",
          args: ["-c", script],
          cwd: MINI_REST_WT,
          timeoutMs: 240_000,
        };
      },
    }),

    cargo_check: shell({
      effect: idempotentEffect("beta8.cargo_check"),
      timeoutMs: 5_400_000,
      exec: () => ({
        command: "bash",
        cwd: MINI_REST_WT,
        timeoutMs: 5_340_000,
        allowNonZeroExit: true,
        maxOutputChars: 60_000,
        args: [
          "-c",
          [
            "set -uo pipefail",
            "FREE_GB=$(df -g /Volumes/Mini-SSD | awk 'NR==2 {print $4}')",
            'if [ "$FREE_GB" -lt 15 ]; then echo "Refusing build: only ${FREE_GB}GB free on Mini-SSD"; exit 2; fi',
            "nice -n 15 cargo check --workspace --bins 2>&1",
          ].join("\n"),
        ],
      }),
      parse: (r) => ({
        ok: r.exitCode === 0,
        exitCode: r.exitCode,
        durationMs: r.durationMs,
        outputTail: `${r.stdout}\n${r.stderr}`.trim().slice(-15_000),
      }),
    }),

    final_review_gate: humanDecision({
      audience: "operator",
      choices: reviewGateChoices,
      request: ({ outputs }) => {
        const check = outputs.cargo_check as {
          ok: boolean;
          exitCode: number | null;
          durationMs: number;
          outputTail: string;
        };
        return {
          title: check.ok
            ? "All 151 conflicts gathered and cargo check PASSED"
            : `All 151 conflicts gathered; cargo check exited ${String(check.exitCode)}`,
          subject: {
            cargoCheck: check,
            group2: outputs.resolve_group2_shell,
            group3: outputs.resolve_group3_pager,
            group4: outputs.resolve_group4_acp_session,
            repair: outputs.repair_compile,
          },
          presentation: {
            schema: "pi-workflows.decision-presentation.v1",
            summary: check.ok
              ? "Workspace compile check succeeded in beta8-rest. Choose Accept to finish or Repair to run another pass."
              : "Inspect the compiler diagnostics below and choose Repair to dispatch Claude Sonnet on the remaining compile errors.",
            blocks: [
              {
                kind: "preformatted",
                text: check.outputTail || "(no compiler output)",
              },
            ],
          },
        };
      },
    }),

    repair_compile: agent({
      agent: "claude",
      model: "sonnet",
      timeoutMs: 3_600_000,
      prompt: ({ outputs }) => {
        const check = outputs.cargo_check as { outputTail: string };
        return [
          `Work strictly inside ${MINI_REST_WT}, where all 151 merge conflicts have been gathered.`,
          "`cargo check --workspace --bins` reported the following compiler errors after combining the four conflict groups:",
          "```",
          check.outputTail,
          "```",
          "Fix the cross-module compile errors while preserving both our fork's intent and upstream v0.1.14-beta.8's intent. Stage any edited files with `git add <file>`. Do NOT commit.",
          'When ready for another cargo check, call the workflow tool to submit {"repaired": true, "notes": "<what you fixed>"}.',
        ].join("\n");
      },
      expectedOutput: '{"repaired": true, "notes": "summary of compile fixes"}',
      validate: (raw: unknown) => {
        const out = raw as { repaired?: unknown; notes?: unknown } | null;
        if (out?.repaired !== true || typeof out.notes !== "string" || out.notes.trim().length === 0) {
          throw new Error('Submit {"repaired": true, "notes": "<what you fixed>"} with non-empty notes.');
        }
        assertNoConflictMarkers(
          MINI_REST_WT,
          [...GROUP1_AIR_PATHS, ...GROUP1_EXTRA_WIRING, ...GROUP2_SHELL_PATHS, ...GROUP3_PAGER_PATHS, ...GROUP4_KEEP_PATHS, GROUP4_SESSION_TARGET],
          GROUP4_DELETE_PATHS,
        );
        return { repaired: true, notes: out.notes.trim() };
      },
    }),

    done: compute({
      run: ({ outputs }) => ({
        host: "mini",
        worktree: MINI_REST_WT,
        cargoCheck: outputs.cargo_check,
      }),
    }),

    stopped: compute({
      run: () => ({ stopped: true }),
    }),
  },
  edges: [
    {
      from: "route_host",
      switch: {
        on: "$.host",
        cases: {
          air: "resolve_group1_air",
          mini: "resolve_group2_shell",
        },
      },
    },
    { from: "resolve_group1_air", to: "air_done" },
    { from: "resolve_group2_shell", to: "resolve_group3_pager" },
    { from: "resolve_group3_pager", to: "resolve_group4_acp_session" },
    { from: "resolve_group4_acp_session", to: "wait_for_air_gate" },
    humanDecisionEdge({
      from: "wait_for_air_gate",
      choices: airGateChoices,
      cases: {
        gather: "gather_resolved_files",
        stop: "stopped",
      },
    }),
    { from: "gather_resolved_files", to: "cargo_check" },
    { from: "cargo_check", to: "final_review_gate" },
    humanDecisionEdge({
      from: "final_review_gate",
      choices: reviewGateChoices,
      cases: {
        done: "done",
        repair: "repair_compile",
      },
    }),
    { from: "repair_compile", to: "cargo_check" },
  ],
});
