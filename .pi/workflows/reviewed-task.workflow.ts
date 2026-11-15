import { execFileSync } from "node:child_process";
import path from "node:path";
import { agent, compute, defineWorkflow } from "@osolmaz/pi-workflows";
import {
  digest,
  parseModel,
  parsePacket,
  parseCandidate,
  parseReview,
  parseReviewBound,
  reviewRoute,
} from "./lib/landing-contracts.mjs";
import independentReview from "./lib/independent-review.workflow.ts";
import { includeWorkflow } from "@osolmaz/pi-workflows";

type Input = {
  cwd: string;
  packet: ReturnType<typeof parsePacket>;
  reviewer: ReturnType<typeof parseModel>;
  reviewRounds: number;
};
const head = (cwd: string) =>
  execFileSync("git", ["rev-parse", "HEAD"], { cwd, encoding: "utf8" }).trim();

export default defineWorkflow({
  source: import.meta.url,
  name: "reviewed-task",
  title: ({ input }) => `Implement ↔ review: ${(input as Input).packet.id}`,
  input: (raw): Input => {
    const v = raw as Input;
    if (!v || !path.isAbsolute(v.cwd)) throw new Error("cwd must be an absolute isolated checkout");
    return {
      cwd: v.cwd,
      packet: parsePacket(v.packet),
      reviewer: parseModel(v.reviewer),
      reviewRounds: parseReviewBound(v.reviewRounds),
    };
  },
  startAt: "implement",
  maxSteps: 80,
  includes: {
    review: includeWorkflow(independentReview, {
      input: ({ input, outputs, state }) => {
        const v = input as Input;
        return {
          cwd: v.cwd,
          goal: v.packet.goal,
          acceptance: v.packet.acceptance,
          context: v.packet.context,
          revision: (outputs.candidate as any).revision,
          model: v.reviewer,
          previous: outputs.review ? (outputs.review as any).output : null,
          label: `${v.packet.id}-review-${state.steps.filter((s) => s.nodeId === "candidate").length}`,
        };
      },
    }),
  },
  nodes: {
    implement: agent({
      timeoutMs: null,
      allowedTools: ["read", "bash", "edit", "write", "codemode"],
      prompt: ({ input, outputs }) => {
        const v = input as Input;
        return [
          `Work only in ${v.cwd}. You are the persistent builder for this approved packet:`,
          JSON.stringify(v.packet),
          `First confirm your actual provider/model are ${v.packet.builder.provider}/${v.packet.builder.model}; otherwise report blocked, never substitute.`,
          "Read the packet's evidence, identify the concrete remaining gaps, then finish only its focused goal. Existing code is the starting point, not a request to rewrite it.",
          "Write/update the few tests and snapshots needed to expose the requested failure modes. Do not accept snapshots blindly.",
          "NO cargo build/check/test, install, push, merge into main, workflow edits, or other agent dispatch. Preserve unrelated work. Commit explicit task files only.",
          "A shared file may be edited in this isolated checkout; the integration owner resolves cross-task overlaps later. Stay within listed paths; a necessary scope change returns blocked with evidence.",
          outputs.review
            ? `Independent review to address: ${JSON.stringify((outputs.review as any).output)}`
            : "",
          "Return {ready:true,candidate:{revision,summary,checks:[{id,command:'cargo',args:[...],timeoutMs}],smoke:[...]}} after committing; checks are minimal scoped cargo test --release commands for later, not commands to run now.",
          "If the goal cannot be completed in scope, return {ready:false,reason:'concrete reason'}; keep accepted work and findings. No invented success.",
        ].join("\n");
      },
      expectedOutput:
        "{ready:true,candidate:{revision,summary,checks,smoke}} or {ready:false,reason:string}",
      validate: (raw, { input }) => {
        const v = raw as any;
        if (v?.ready === false && typeof v.reason === "string" && v.reason.trim()) return v;
        if (v?.ready !== true) throw new Error("Explicit ready or blocked result required");
        const candidate = parseCandidate(v.candidate);
        if (head((input as Input).cwd) !== candidate.revision)
          throw new Error("Candidate must identify current HEAD");
        if (
          execFileSync("git", ["status", "--porcelain", "--", ".", ":!.worktrees/"], {
            cwd: (input as Input).cwd,
            encoding: "utf8",
          }).trim()
        )
          throw new Error("Commit all task changes before review");
        return { ready: true, candidate };
      },
    }),
    implemented: compute({
      run: ({ outputs }) => ({ route: (outputs.implement as any).ready ? "candidate" : "blocked" }),
    }),
    candidate: compute({ run: ({ outputs }) => (outputs.implement as any).candidate }),
    route: compute({
      run: ({ input, outputs, state }) => {
        const v = input as Input;
        const review = parseReview((outputs.review as any).output);
        const candidate = outputs.candidate as any;
        const round = state.steps.filter((s) => s.nodeId === "candidate").length;
        return {
          route: reviewRoute(
            review,
            candidate.revision,
            digest({ goal: v.packet.goal, acceptance: v.packet.acceptance }),
            round,
            v.reviewRounds,
          ),
          round,
        };
      },
    }),
    ready: compute({
      run: ({ input, outputs, state }) => ({
        status: "ready",
        task: (input as Input).packet.id,
        packetDigest: digest((input as Input).packet),
        cwd: (input as Input).cwd,
        candidate: outputs.candidate,
        review: (outputs.review as any).output,
        rounds: state.steps.filter((s) => s.nodeId === "candidate").length,
        runId: state.runId,
      }),
    }),
    blocked: compute({
      run: ({ input, outputs, results, state }) => ({
        status: "blocked",
        task: (input as Input).packet.id,
        cwd: (input as Input).cwd,
        candidate: outputs.candidate ?? null,
        findings: outputs.review ?? outputs.implement ?? results.implement,
        reason:
          "Implementation/review did not reach readiness within the approved scope and review bound. Preserve this handoff for a separately scoped repair.",
        runId: state.runId,
      }),
    }),
  },
  exits: { ready: { from: "ready" }, blocked: { from: "blocked" } },
  edges: [
    {
      from: "implement",
      switch: {
        on: "$result.outcome",
        cases: { ok: "implemented", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "implemented",
      switch: { on: "$.route", cases: { candidate: "candidate", blocked: "blocked" } },
    },
    { from: "candidate", to: "review" },
    { from: "review.ready", to: "route" },
    { from: "review.changes", to: "route" },
    { from: "review.blocked", to: "blocked" },
    {
      from: "route",
      switch: { on: "$.route", cases: { ready: "ready", repair: "implement", blocked: "blocked" } },
    },
  ],
});
