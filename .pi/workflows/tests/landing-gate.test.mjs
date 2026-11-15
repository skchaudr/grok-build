// Isolated database only: exercise actual human gate persistence and resume.
import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { verifyApprovedBundle } from "../lib/approved-packets.mjs";
const root = process.env.PI_WORKFLOWS_ROOT;
const { loadWorkflowFile } = await import(path.join(root, "src/workflows/loader.ts"));
const { WorkflowEngine } = await import(path.join(root, "src/workflows/engine.ts"));
const { WorkflowRunStore } = await import(path.join(root, "src/workflows/store.ts"));
const { HumanDecisionStore } = await import(path.join(root, "src/workflows/human-decision.ts"));
const { requestForAttempt, recordWorkflowSubmission } = await import(
  path.join(root, "src/workflows/requests.ts")
);
const workflow = await loadWorkflowFile(
  new URL("../prepare-landing.workflow.ts", import.meta.url).pathname,
);
for (const choice of ["approve", "revise", "stop"])
  test(`actual protected packet gate: ${choice}, same-run resume`, async () => {
    const cwd = mkdtempSync(path.join(tmpdir(), "landing-gate-"));
    execFileSync("git", ["init", "-q", cwd]);
    execFileSync("git", [
      "-C",
      cwd,
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "commit",
      "--allow-empty",
      "-qm",
      "fixture",
    ]);
    const base = execFileSync("git", ["-C", cwd, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    const model = { provider: "fixture", model: "fixture", thinking: "high" };
    const input = {
      cwd,
      goal: "Land three features",
      context: ["fixture"],
      planner: model,
      builders: Object.fromEntries(["f3", "cass", "claude-settings"].map((id) => [id, model])),
      reviewer: model,
      evaluator: model,
      reviewRounds: 5,
    };
    let detailCalls = 0;
    const executor = {
      enforcesToolAllowlist: true,
      async runAgentStep(r) {
        detailCalls++;
        const value = {
          base,
          notes: "fixture proposal",
          packets: ["f3", "cass", "claude-settings"].map((id) => ({
            id,
            goal: id,
            context: ["fixture"],
            files: ["src"],
            constraints: ["No cargo"],
            acceptance: ["Goal observed"],
            startingRef: base,
            builder: model,
          })),
        };
        const result = await r.accept(value);
        assert.equal(result.ok, true, result.error);
        return { output: result.value };
      },
    };
    const store = new WorkflowRunStore(path.join(cwd, "test.sqlite"));
    try {
      const engine = new WorkflowEngine({ store, executor });
      const waiting = await engine.run(workflow, input);
      assert.equal(waiting.state.status, "waiting");
      assert.equal(detailCalls, 1);
      const req = requestForAttempt(store.state, waiting.runId, waiting.state.currentAttemptId);
      assert.equal(req.kind, "decision");
      assert.throws(
        () =>
          recordWorkflowSubmission(store.state, {
            requestId: req.requestId,
            submissionId: "forged",
            idempotencyKey: "forged",
            expectedRevision: req.revision,
            payload: { choice: "approve" },
            outcome: "accepted",
            settle: true,
          }),
        /verified human decision/,
      );
      const decision = req.contract;
      const response =
        choice === "revise"
          ? { choice, input: { instructions: "Clarify failure outcome" } }
          : { choice };
      const accepted = await new HumanDecisionStore(store.databasePath, {
        state: store.state,
      }).accept(decision, {
        decisionId: decision.decisionId,
        requestDigest: decision.requestDigest,
        ...response,
        source: { channel: "pi", actorId: "fixture-human", eventId: "fixture-event" },
        idempotencyKey: "fixture-event",
      });
      recordWorkflowSubmission(store.state, {
        requestId: req.requestId,
        submissionId: "human",
        idempotencyKey: "human",
        expectedRevision: req.revision,
        payload: accepted.decision.response,
        outcome: "accepted",
        settle: true,
      });
      const resumed = await engine.resumeRun(workflow, waiting.runId);
      assert.equal(resumed.runId, waiting.runId);
      if (choice === "revise") {
        assert.equal(resumed.state.status, "waiting");
        assert.equal(detailCalls, 2);
      } else {
        assert.equal(resumed.state.status, "completed");
        assert.equal(
          resumed.state.finalOutput.status,
          choice === "approve" ? "approved" : "stopped",
        );
        if (choice === "approve") {
          const bundle = JSON.parse(readFileSync(resumed.state.finalOutput.file, "utf8"));
          assert.equal(bundle.approval.choice, "approve");
          assert.equal(bundle.packets.length, 3);
          await verifyApprovedBundle(bundle, {
            async getRun() {
              return { state: resumed.state };
            },
            async hydrateContent(_, value) {
              return value;
            },
          });
        }
      }
    } finally {
      store.close();
      rmSync(cwd, { recursive: true, force: true });
    }
  });
