// Deterministic actual-engine tests: isolated temp SQLite, no model calls, no live server.
import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { digest } from "../lib/landing-contracts.mjs";
const root = process.env.PI_WORKFLOWS_ROOT;
if (!root) throw new Error("PI_WORKFLOWS_ROOT required");
const { loadWorkflowFile } = await import(path.join(root, "src/workflows/loader.ts"));
const { compileWorkflowDefinition } = await import(path.join(root, "src/workflows/composition.ts"));
const { WorkflowEngine } = await import(path.join(root, "src/workflows/engine.ts"));
const task = await loadWorkflowFile(
  new URL("../reviewed-task.workflow.ts", import.meta.url).pathname,
);

for (const scenario of ["ready", "repair-ready", "exhausted", "review-failed", "cancelled"])
  test(`actual engine: ${scenario}`, async () => {
    const cwd = mkdtempSync(path.join(tmpdir(), "landing-engine-"));
    const git = (...a) => execFileSync("git", a, { cwd, encoding: "utf8" }).trim();
    git("init", "-q");
    writeFileSync(path.join(cwd, ".gitignore"), ".pi/lane/\nstate.sqlite*\n");
    git("add", ".gitignore");
    git(
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "commit",
      "--allow-empty",
      "-qm",
      "fixture",
    );
    const revision = git("rev-parse", "HEAD");
    const packet = {
      id: "f3",
      goal: "Show usage",
      context: ["fixture"],
      files: ["fixture.rs"],
      constraints: ["No cargo"],
      acceptance: ["Usage shown"],
      startingRef: revision,
      builder: { provider: "fixture", model: "writer", thinking: "high" },
    };
    const candidate = {
      revision,
      summary: "fixture",
      checks: [
        {
          id: "usage",
          command: "cargo",
          args: ["test", "--release", "-p", "xai-grok-pager", "usage"],
          timeoutMs: 1000,
        },
      ],
      smoke: ["Inspect usage"],
    };
    let reviews = 0,
      implementations = 0;
    const executor = {
      enforcesToolAllowlist: true,
      async runAgentStep(request) {
        const id = request.contract.nodeId;
        if (id === "implement") {
          implementations++;
          const r = await request.accept({ ready: true, candidate });
          if (!r.ok) throw new Error(r.error);
          return { output: r.value };
        }
        if (id === "review/review") {
          reviews++;
          if (scenario === "review-failed") throw new Error("synthetic provider failure");
          if (scenario === "cancelled")
            throw Object.assign(new Error("cancelled fixture"), { name: "AbortError" });
          const file = request.prompt.match(/Write only ([^:]+\.json): JSON/)[1];
          const ready = scenario === "ready" || (scenario === "repair-ready" && reviews === 2);
          writeFileSync(
            file,
            JSON.stringify({
              revision,
              goalDigest: digest({ goal: packet.goal, acceptance: packet.acceptance }),
              verdict: ready ? "ready" : "changes",
              findings: ready
                ? []
                : [{ id: "F1", severity: "P1", file: "fixture.rs", message: "missing usage" }],
              summary: "fixture review",
            }),
          );
          const r = await request.accept({ file });
          if (!r.ok) throw new Error(r.error);
          return { output: r.value };
        }
        throw new Error(`Unexpected model stage ${id}`);
      },
    };
    try {
      const engine = new WorkflowEngine({ executor, databasePath: path.join(cwd, "state.sqlite") });
      const { state } = await engine.run(compileWorkflowDefinition(task), {
        cwd,
        packet,
        reviewer: { provider: "fixture", model: "reviewer", thinking: "high" },
        reviewRounds: 2,
      });
      if (scenario === "ready" || scenario === "repair-ready")
        assert.equal(state.finalOutput.status, "ready");
      else if (scenario === "cancelled") {
        assert.equal(state.status, "cancelled");
        assert.equal(state.finalOutput, undefined);
      } else assert.equal(state.finalOutput?.status, "blocked");
      assert.equal(reviews, scenario === "repair-ready" || scenario === "exhausted" ? 2 : 1);
      assert.equal(
        implementations,
        scenario === "repair-ready" || scenario === "exhausted" ? 2 : 1,
      );
    } finally {
      rmSync(cwd, { recursive: true, force: true });
    }
  });
