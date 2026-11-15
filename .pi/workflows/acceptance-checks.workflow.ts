import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { action, agent, defineWorkflow, manualEffect } from "@osolmaz/pi-workflows";
import { parseCandidate, parseModel } from "./lib/landing-contracts.mjs";

export default defineWorkflow({
  source: import.meta.url,
  name: "acceptance-checks",
  title: "Independent, minimal goal-driven checks",
  input: (raw) => {
    const v = raw as any;
    if (
      !v ||
      !path.isAbsolute(v.cwd) ||
      !path.isAbsolute(v.output) ||
      !v.goal?.trim() ||
      !Array.isArray(v.goals) ||
      !v.goals.length
    )
      throw new Error("Need checkout, output path, overall goal and acceptance-only goals");
    return {
      cwd: v.cwd,
      output: v.output,
      goal: v.goal,
      goals: v.goals,
      model: parseModel(v.model),
    };
  },
  startAt: "design_checks",
  maxSteps: 3,
  nodes: {
    design_checks: agent({
      timeoutMs: null,
      allowedTools: ["read", "bash", "codemode"],
      prompt: ({ input }) => {
        const v = input as any;
        return [
          `Confirm actual model ${v.model.provider}/${v.model.model}; no substitution. Work read-only in ${v.cwd}.`,
          `Overall goal: ${v.goal}. Feature goals and required observable outcomes: ${JSON.stringify(v.goals)}.`,
          "You have deliberately not been given builder reports or patches. Start by deciding what small number of checks would expose failure to meet these goals, before inspecting existing test interfaces.",
          "Inspect only enough existing code/tests to identify how each check can run. No broad repo audit, code edits, Cargo, build, installs, dispatch or acceptance weakening.",
          "Return {revision:'checkout HEAD',summary:'goal-driven rationale; precise missing test cases the integration owner must add',checks:[{id,command:'cargo',args:[...scoped release-test command...],timeoutMs}],smoke:[few real screen checks]}.",
          "Keep it minimal and direct: checks must fail if a named user outcome fails. Mark missing tests explicitly in summary; existing passing tests are not substitutes for missing outcomes. The integration parent must implement missing tests before final review.",
          "Each command must be cargo test --release -p xai-grok-pager|xai-grok-pager-bin|xai-grok-traces with an explicit scoped test filter. Do not run it here.",
        ].join("\n");
      },
      expectedOutput: "{revision:string,summary:string,checks:[],smoke:[]}",
      validate: (raw) => parseCandidate(raw),
    }),
    save: action({
      effect: manualEffect("landing.acceptance-handoff"),
      run: ({ input, outputs, state }) => {
        const v = input as any;
        mkdirSync(path.dirname(v.output), { recursive: true });
        const result = {
          status: "designed",
          goal: v.goal,
          goals: v.goals,
          design: outputs.design_checks,
          runId: state.runId,
        };
        writeFileSync(v.output, JSON.stringify(result, null, 2) + "\n", { flag: "wx" });
        return { ...result, file: v.output };
      },
    }),
  },
  edges: [{ from: "design_checks", to: "save" }],
});
