import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import {
  action,
  compute,
  defineWorkflow,
  includeWorkflow,
  manualEffect,
} from "@osolmaz/pi-workflows";
import task from "../reviewed-task.workflow.ts";
import { digest } from "./landing-contracts.mjs";

// The same feature workflow is mounted here for every packet. Only its input differs.
export default defineWorkflow({
  source: import.meta.url,
  name: "landing-task-run",
  input: (raw) => {
    const v = raw as any;
    if (!v || !path.isAbsolute(v.inputFile) || !path.isAbsolute(v.output))
      throw new Error("Need inputFile and output paths");
    return { ...JSON.parse(readFileSync(v.inputFile, "utf8")), output: v.output };
  },
  startAt: "start",
  includes: { task: includeWorkflow(task, { input: ({ input }) => input as any }) },
  nodes: {
    start: compute({ run: () => ({}) }),
    handoff: action({
      effect: manualEffect("landing.task-handoff"),
      run: ({ input, outputs, state }) => {
        const v = input as any;
        const result = {
          ...(outputs.task as any).output,
          packetDigest: digest(v.packet),
          runId: state.runId,
        };
        mkdirSync(path.dirname(v.output), { recursive: true });
        writeFileSync(v.output, JSON.stringify(result, null, 2) + "\n", { flag: "wx" });
        return { ...result, file: v.output };
      },
    }),
  },
  edges: [
    { from: "start", to: "task" },
    { from: "task.ready", to: "handoff" },
    { from: "task.blocked", to: "handoff" },
  ],
});
