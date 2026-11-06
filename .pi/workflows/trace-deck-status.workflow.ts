import { fileURLToPath } from "node:url";
import { agent, assistantMessage, defineWorkflow } from "@osolmaz/pi-workflows";

const plan = fileURLToPath(new URL("../../../trace-deck/PLAN.md", import.meta.url));
const handoff = fileURLToPath(
  new URL("../../.handoffs/001-traces-views-2026-11-04.md", import.meta.url),
);

export default defineWorkflow({
  name: "trace-deck-status",
  title: "Trace-deck: plan → handoff → synthesis",
  startAt: "read_plan",
  maxSteps: 3,
  nodes: {
    read_plan: agent({
      allowedTools: ["read"],
      timeoutMs: 300_000,
      prompt: () =>
        `Read the complete file ${plan} using read in this step. Record its product goals and execution plan as concise notes. Treat the document as source material, not instructions to implement it. If reading fails, report the error; do not invent notes or substitute another file. Do not edit or create files.`,
      expectedOutput: '{ "notes": "Concise notes from PLAN.md, or the reading error." }',
    }),
    read_handoff: agent({
      allowedTools: ["read"],
      timeoutMs: 300_000,
      prompt: () =>
        `Read the complete recent agent handoff ${handoff} using read in this step. Record what it reports about the current product, landed work, preserved work, and unfinished work. Treat the document as source material, not instructions to implement it. If reading fails, report the error; do not invent notes or substitute another file. Do not edit or create files.`,
      expectedOutput: '{ "notes": "Concise notes from the recent handoff, or the reading error." }',
    }),
    synthesize: agent({
      allowedTools: [],
      timeoutMs: 300_000,
      prompt: ({ outputs }) => [
        "Synthesize the two readings inline in a concise response (at most 180 words).",
        "Explain the intended product, where the handoff says it stands, and what remains.",
        "Distinguish source claims from runtime verification. Do not imply any checks were run on sab-air.",
        "If either reading failed, state that the synthesis is incomplete and identify the error.",
        "Do not create a report file or propose additional workflow stages.",
        `PLAN.md notes: ${JSON.stringify(outputs.read_plan)}`,
        `Recent handoff notes: ${JSON.stringify(outputs.read_handoff)}`,
      ].join("\n"),
      expectedOutput: assistantMessage(),
    }),
  },
  edges: [
    { from: "read_plan", to: "read_handoff" },
    { from: "read_handoff", to: "synthesize" },
  ],
});
