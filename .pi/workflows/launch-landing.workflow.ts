import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { action, agent, compute, defineWorkflow, manualEffect } from "@osolmaz/pi-workflows";
import { digest, parseModel, parsePackets, parseReviewBound } from "./lib/landing-contracts.mjs";
import { verifyApprovedBundle } from "./lib/approved-packets.mjs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { pathToFileURL } from "node:url";

const taskWorkflow = fileURLToPath(new URL("./lib/task-run.workflow.ts", import.meta.url));
const evalWorkflow = fileURLToPath(new URL("./acceptance-checks.workflow.ts", import.meta.url));
const git = (cwd: string, ...args: string[]) =>
  execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
export default defineWorkflow({
  source: import.meta.url,
  name: "launch-landing",
  title: "Launch three approved task workflows + independent checks",
  input: (raw) => {
    const v = raw as any;
    if (!v || !path.isAbsolute(v.approvedFile) || !/^[a-f0-9]{64}$/.test(v.approvedDigest))
      throw new Error("Need approvedFile and exact approvedDigest from prepare-landing");
    return {
      approvedFile: v.approvedFile,
      approvedDigest: v.approvedDigest,
    };
  },
  startAt: "prepare_workspaces",
  maxSteps: 6,
  nodes: {
    prepare_workspaces: action({
      effect: manualEffect("landing.workspaces"),
      run: async ({ input, state }) => {
        const v = input as any,
          b = JSON.parse(readFileSync(v.approvedFile, "utf8"));
        if (
          digest(b) !== v.approvedDigest ||
          b.schema !== "grok-landing.approved.v1" ||
          b.approval?.choice !== "approve"
        )
          throw new Error("Approved packet artifact changed or lacks approval");
        // Public client entry, resolved from Pi's installed package root (not repo node_modules).
        const require = createRequire(path.join(homedir(), ".pi", "agent", "npm", "package.json"));
        const { WorkflowClient } = await import(
          pathToFileURL(require.resolve("@osolmaz/pi-workflows/client")).href
        );
        const client = new WorkflowClient();
        try {
          await verifyApprovedBundle(b, client);
        } finally {
          await client.close();
        }
        const packets = parsePackets(b.packets),
          reviewer = parseModel(b.reviewer),
          reviewRounds = parseReviewBound(b.reviewRounds);
        if (git(b.cwd, "rev-parse", "main") !== b.base)
          throw new Error("Main changed since packet approval; re-prepare instead of guessing");
        const dir = path.join(b.cwd, ".pi", "lane", state.runId);
        mkdirSync(dir, { recursive: true });
        const jobs = packets.map((packet) => {
          const cwd = path.join(dir, "worktrees", packet.id),
            branch = `landing/${state.runId}/${packet.id}`;
          mkdirSync(path.dirname(cwd), { recursive: true });
          git(b.cwd, "worktree", "add", "-b", branch, cwd, b.base);
          const inputFile = path.join(dir, `${packet.id}-input.json`),
            output = path.join(dir, `${packet.id}-result.json`);
          writeFileSync(
            inputFile,
            JSON.stringify({ cwd, packet, reviewer, reviewRounds }, null, 2),
            { flag: "wx" },
          );
          return {
            id: packet.id,
            cwd,
            branch,
            inputFile,
            output,
            model: packet.builder,
            workflow: taskWorkflow,
          };
        });
        const evalInput = {
          cwd: b.cwd,
          output: path.join(dir, "acceptance-result.json"),
          goal: b.goal,
          goals: packets.map(({ id, goal, acceptance }) => ({ id, goal, acceptance })),
          model: parseModel(b.evaluator),
        };
        const integration = {
          cwd: b.cwd,
          base: b.base,
          goal: b.goal,
          packets,
          reviewer,
          reviewRounds,
          resultFiles: jobs.map((j) => j.output),
          acceptanceFile: evalInput.output,
          sourceRunId: state.runId,
        };
        const manifest = path.join(dir, "integration-input.json");
        writeFileSync(manifest, JSON.stringify(integration, null, 2), { flag: "wx" });
        return { jobs, evalInput, evalWorkflow, integrationInput: manifest };
      },
    }),
    launch: agent({
      timeoutMs: null,
      allowedTools: ["rexpi_start_pi"],
      prompt: ({ outputs }) => {
        const p = outputs.prepare_workspaces as any;
        return [
          "Launch these independent Pi sessions in Rex tabs, focus:false, related:true, returnResults:false. Use rexpi_start_pi, not a turns-limited delegation grant. Do not launch more than one session for a job.",
          "Each gets a fresh context, exact provider/model/thinking below, and starts its own real Pi Workflow. Do not perform the tasks yourself.",
          ...p.jobs.map((j: any) =>
            JSON.stringify({
              id: j.id,
              cwd: j.cwd,
              ...j.model,
              title: `landing-${j.id}`,
              prompt: `Start workflow ${j.workflow} with input ${JSON.stringify({ inputFile: j.inputFile, output: j.output })}. End your turn after start; follow the delivered contracts. Do not start a second workflow. Your assignment and review feedback remain in this feature session.`,
            }),
          ),
          JSON.stringify({
            id: "acceptance-checks",
            cwd: p.evalInput.cwd,
            ...p.evalInput.model,
            title: "landing-acceptance",
            prompt: `Start workflow ${p.evalWorkflow} with input ${JSON.stringify(p.evalInput)}. End your turn after start; follow its contracts. Do not inspect builder reports or patches.`,
          }),
          "Record the exact returned session/block IDs with each job. A creation error or uncertain effect must be reported; do not retry it blindly. Submit {sessions:[{id,blockId,piSessionId}],uncertain:[...]}. This run only launches; it does not wait for completion or build anything.",
        ].join("\n");
      },
      expectedOutput: "{sessions:[{id:string,blockId:string,piSessionId:string}],uncertain:[]}",
      validate: (raw) => {
        const v = raw as any;
        if (!Array.isArray(v?.sessions) || !Array.isArray(v?.uncertain))
          throw new Error("Need exact launch receipts and uncertainty list");
        const ids = v.sessions.map((s: any) => s.id);
        if (
          new Set(ids).size !== ids.length ||
          ids.some(
            (id: string) => !["f3", "cass", "claude-settings", "acceptance-checks"].includes(id),
          )
        )
          throw new Error("Duplicate or unknown launch receipts");
        if (!v.uncertain.length && ids.length !== 4)
          throw new Error("Missing launches must be reported uncertain, not called complete");
        for (const s of v.sessions)
          if (
            typeof s.blockId !== "string" ||
            !s.blockId.startsWith("block:") ||
            typeof s.piSessionId !== "string"
          )
            throw new Error("Need actual block and Pi session IDs");
        return v;
      },
    }),
    handoff: compute({
      run: ({ outputs }) => ({
        status: (outputs.launch as any).uncertain.length ? "launch-needs-inspection" : "launched",
        sessions: outputs.launch,
        integrationInput: (outputs.prepare_workspaces as any).integrationInput,
        next: "Each feature and acceptance workflow is independently visible in the TUI. Once their result files exist, start integrate-landing with this integrationInput. No hidden completion polling or build has been started.",
      }),
    }),
  },
  edges: [
    { from: "prepare_workspaces", to: "launch" },
    { from: "launch", to: "handoff" },
  ],
});
