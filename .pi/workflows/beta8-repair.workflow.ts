import { spawnSync } from "node:child_process";
import {
  agent,
  allowSettingsPath,
  choice,
  compute,
  defineHumanChoices,
  defineWorkflow,
  humanDecision,
  humanDecisionEdge,
  idempotentEffect,
  settingsRoute,
  shell,
  workflowSettings,
} from "@osolmaz/pi-workflows";

// Bounded Sonnet repair loop for the gathered beta8 merge tree.
// check fails -> guard -> repair (at most maxRepairs times) -> check ...; then you decide at the gate.
const WORKTREE = "/Volumes/Mini-SSD/offload/repos/grok-build/.worktrees/beta8-rest";

type Settings = { maxRepairs: number };
type RepairOutput = { notes: string };

const settings = workflowSettings({
  initial: { maxRepairs: 3 },
  parse: (value) => {
    const v = value as Partial<Settings> | null;
    if (!v || !Number.isInteger(v.maxRepairs) || (v.maxRepairs as number) < 0) {
      throw new Error("maxRepairs must be a non-negative integer");
    }
    return { maxRepairs: v.maxRepairs as number };
  },
  paths: [allowSettingsPath("/maxRepairs", { read: ["human"], replace: ["human"] })],
});

const gateChoices = defineHumanChoices({
  accept: choice({ label: "Accept the tree" }),
  repair: choice({ label: "One more Sonnet repair" }),
  stop: choice({ label: "Stop" }),
});

function git(args: string[]): { status: number | null; stdout: string } {
  const r = spawnSync("git", args, { cwd: WORKTREE, encoding: "utf8" });
  return { status: r.status, stdout: r.stdout ?? "" };
}

export default defineWorkflow({
  name: "beta8-repair",
  title: "Repair beta8-rest until cargo check passes",
  settings,
  startAt: "disk",
  maxSteps: 40,
  nodes: {
    // Stops the run (no failure route) instead of handing a full disk to Sonnet.
    disk: shell({
      effect: idempotentEffect("beta8-repair.disk"),
      timeoutMs: 30_000,
      exec: () => ({
        command: "bash",
        cwd: WORKTREE,
        args: [
          "-c",
          "FREE_GB=$(df -g /Volumes/Mini-SSD | awk 'NR==2 {print $4}'); " +
            'if [ "$FREE_GB" -lt 15 ]; then echo "only ${FREE_GB}GB free on Mini-SSD" >&2; exit 2; fi; ' +
            'echo "${FREE_GB}GB free"',
        ],
      }),
    }),

    // Non-zero exit fails the node, so the card shows the failure; diagnostics go to stderr -> result.error.
    check: shell({
      effect: idempotentEffect("beta8-repair.check"),
      timeoutMs: 5_400_000,
      exec: () => ({
        command: "nice",
        args: ["-n", "15", "cargo", "check", "--workspace", "--bins", "--message-format", "short"],
        cwd: WORKTREE,
      }),
    }),

    guard: settingsRoute({
      run: ({ state, settings }) => {
        const used = state.steps.filter((s) => s.nodeId === "repair").length;
        return { route: used < (settings as Settings).maxRepairs ? "retry" : "max", used };
      },
    }),

    repair: agent({
      agent: "claude",
      model: "sonnet",
      timeoutMs: 3_600_000,
      prompt: ({ results, state }) => {
        const earlier = state.steps
          .filter((s) => s.nodeId === "repair" && s.outcome === "ok")
          .map((s, i) => `${i + 1}. ${(s.output as RepairOutput).notes}`);
        return [
          `Work strictly inside ${WORKTREE}. It holds the gathered Grok Build + Dwsy v0.1.14-beta.8 merge (all 151 conflicts resolved and staged).`,
          "`cargo check --workspace --bins` fails with:",
          "```",
          results.check?.error ?? "(no diagnostics captured; run cargo check yourself)",
          "```",
          ...(earlier.length > 0 ? ["Earlier repair passes in this run:", ...earlier, ""] : []),
          "Fix the compile errors. Keep both sides' intent: our fork's multi-host, traces, acpx, piw, Pi/Claude notification routing and isolated grok-home wiring, plus upstream beta.8's additions (mention catalogs, working messages, timeline, thinking_stream_mode, tool-call density, emphasis).",
          "Do not delete or stub out features to make it compile. Edit only what the errors need. Stage edited files with `git add <file>`. Do NOT commit.",
          "You may run `cargo check --workspace --bins --message-format short` yourself.",
          'When done, submit {"notes": "<what you fixed, and anything you could not fix>"}.',
        ].join("\n");
      },
      expectedOutput: '{"notes": "what was fixed"}',
      validate: (raw: unknown): RepairOutput => {
        const out = raw as Partial<RepairOutput> | null;
        if (!out || typeof out.notes !== "string" || out.notes.trim().length === 0) {
          throw new Error('Submit {"notes": "<what you fixed>"} with non-empty notes.');
        }
        const unmerged = git(["diff", "--name-only", "--diff-filter=U"]).stdout.trim();
        if (unmerged) throw new Error(`Unmerged files remain:\n${unmerged}`);
        const markers = git(["grep", "-lE", "^(<<<<<<<|>>>>>>>)( |$)", "--", "crates", "Cargo.toml"]);
        if (markers.status === 0) throw new Error(`Conflict markers in:\n${markers.stdout.trim()}`);
        return { notes: out.notes.trim() };
      },
    }),

    gate: humanDecision({
      audience: "operator",
      choices: gateChoices,
      request: ({ results, state }) => {
        const passed = results.check?.outcome === "ok";
        const repairs = state.steps.filter((s) => s.nodeId === "repair");
        const notes = repairs
          .filter((s) => s.outcome === "ok")
          .map((s, i) => `${i + 1}. ${(s.output as RepairOutput).notes}`)
          .join("\n");
        return {
          title: passed
            ? `cargo check passed after ${repairs.length} repair(s)`
            : `cargo check still failing after ${repairs.length} repair(s)`,
          subject: { check: results.check, repair: results.repair },
          presentation: {
            schema: "pi-workflows.decision-presentation.v1",
            summary: passed ? "Accept, or stop." : "Another Sonnet repair, or stop.",
            blocks: [
              ...(notes ? [{ kind: "preformatted" as const, text: notes }] : []),
              { kind: "preformatted" as const, text: results.check?.error ?? "(no errors)" },
            ],
          },
        };
      },
    }),

    completed: compute({ run: ({ results }) => ({ ok: true, worktree: WORKTREE, check: results.check?.outcome }) }),
    stopped: compute({ run: () => ({ ok: false, worktree: WORKTREE }) }),
  },
  edges: [
    { from: "disk", to: "check" },
    { from: "check", switch: { on: "$result.outcome", cases: { ok: "gate", failed: "guard", timed_out: "guard" } } },
    { from: "guard", switch: { on: "$.route", cases: { retry: "repair", max: "gate" } } },
    { from: "repair", switch: { on: "$result.outcome", cases: { ok: "disk", failed: "gate", timed_out: "gate" } } },
    humanDecisionEdge({
      from: "gate",
      choices: gateChoices,
      cases: { accept: "completed", repair: "repair", stop: "stopped" },
    }),
  ],
});
