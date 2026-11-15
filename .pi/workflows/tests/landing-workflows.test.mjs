import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { passingTests } from "../lib/verify-landing.mjs";

// Run with the installed pi-workflows loader, not a second workflow implementation.
const root = process.env.PI_WORKFLOWS_ROOT;
if (!root)
  throw new Error("Set PI_WORKFLOWS_ROOT to the installed/source pi-workflows package path");
const { loadWorkflowFile } = await import(path.join(root, "src/workflows/loader.ts"));
const { validateWorkflowDefinition, resolveNext } = await import(
  path.join(root, "src/workflows/graph.ts")
);
const { compileWorkflowDefinition } = await import(path.join(root, "src/workflows/composition.ts"));
const here = new URL("../", import.meta.url).pathname;
const names = [
  "prepare-landing",
  "reviewed-task",
  "acceptance-checks",
  "launch-landing",
  "integrate-landing",
  "lib/task-run",
  "lib/independent-review",
  "lib/visible-command",
];
const workflows = Object.fromEntries(
  await Promise.all(
    names.map(async (n) => [n, await loadWorkflowFile(path.join(here, `${n}.workflow.ts`))]),
  ),
);
for (const name of names)
  test(`${name}: actual loader and composition graph validate`, () => {
    validateWorkflowDefinition(workflows[name]);
    assert.ok(Object.keys(compileWorkflowDefinition(workflows[name]).nodes).length);
  });

test("packet decision is a protected human gate without automatic timeout", () => {
  const w = workflows["prepare-landing"],
    gate = w.nodes.packet_gate;
  assert.equal(gate.nodeType, "checkpoint");
  assert.ok(gate.humanDecision);
  assert.equal(gate.humanDecision.onTimeout, undefined);
  const edge = w.edges.find((e) => e.from === "packet_gate");
  assert.deepEqual(
    Object.values(edge.switch.cases).sort(),
    ["approved", "detail_packets", "stopped"].sort(),
  );
});
test("candidate repair returns to persistent builder; final audit changes do not ship", () => {
  const task = workflows["reviewed-task"];
  assert.equal(resolveNext(task.edges, "route", { route: "repair" }), "implement");
  assert.equal(resolveNext(task.edges, "route", { route: "blocked" }), "blocked");
  const integrate = workflows["integrate-landing"];
  assert.equal(resolveNext(integrate.edges, "final_review.changes", {}), "blocked");
  assert.equal(resolveNext(integrate.edges, "final_review.ready", {}), "run_tests");
  assert.equal(resolveNext(integrate.edges, "test_route", { ok: false }), "blocked");
});
test("failed/timeout reviewer cannot route to ready and cancellation remains terminal", () => {
  const w = workflows["lib/independent-review"];
  const e = w.edges.find((e) => e.from === "review");
  assert.equal(e.switch.cases.failed, "blocked");
  assert.equal(e.switch.cases.timed_out, "blocked");
  assert.equal(e.switch.cases.cancelled, undefined);
});
test("review handoff binds exact revision and goal; stale files are rejected", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "landing-review-"));
  try {
    execFileSync("git", ["init", "-q", dir]);
    execFileSync("git", [
      "-C",
      dir,
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "commit",
      "--allow-empty",
      "-qm",
      "fixture",
    ]);
    const revision = execFileSync("git", ["-C", dir, "rev-parse", "HEAD"], {
      encoding: "utf8",
    }).trim();
    mkdirSync(path.join(dir, ".git", "reviews"), { recursive: true });
    const file = path.join(dir, ".git", "reviews", "review.json"),
      goalDigest = "b".repeat(64);
    const context = { input: { cwd: dir, revision }, outputs: { prepare: { file, goalDigest } } };
    const validate = workflows["lib/independent-review"].nodes.review.validate;
    writeFileSync(
      file,
      JSON.stringify({ revision, goalDigest, verdict: "ready", findings: [], summary: "checked" }),
    );
    assert.equal(validate({ file }, context).review.verdict, "ready");
    writeFileSync(
      file,
      JSON.stringify({
        revision: "c".repeat(40),
        goalDigest,
        verdict: "ready",
        findings: [],
        summary: "stale",
      }),
    );
    assert.throws(() => validate({ file }, context), /Stale/);
    assert.throws(
      () => validate({ file: path.join(dir, "other") }, context),
      /exact review artifact/,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test("test/install execution is runtime-owned and final gate shows audit evidence", () => {
  const w = workflows["integrate-landing"];
  assert.equal(w.nodes.tests.nodeType, "compute");
  assert.equal(w.nodes.install.nodeType, "compute");
  const command = workflows["lib/visible-command"].nodes.command;
  assert.equal(command.nodeType, "action");
  assert.equal(typeof command.exec, "function");
  const presentation = w.nodes.deliver_gate.humanDecision.request({
    outputs: {
      candidate: { revision: "a".repeat(40), summary: "candidate" },
      final_review: { audit: "review evidence" },
      tests: { ok: true },
    },
  });
  assert.match(JSON.stringify(presentation.presentation), /review evidence/);
});

test("uncommitted new tests cannot be omitted from a candidate", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "landing-untracked-"));
  try {
    execFileSync("git", ["init", "-q", dir]);
    execFileSync("git", [
      "-C",
      dir,
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "commit",
      "--allow-empty",
      "-qm",
      "fixture",
    ]);
    const revision = execFileSync("git", ["-C", dir, "rev-parse", "HEAD"], {
      encoding: "utf8",
    }).trim();
    writeFileSync(path.join(dir, "new_test.rs"), "// omitted test");
    const candidate = {
      revision,
      summary: "fixture",
      checks: [
        {
          id: "goal",
          command: "cargo",
          args: ["test", "--release", "-p", "xai-grok-pager", "goal"],
          timeoutMs: 1000,
        },
      ],
      smoke: [],
    };
    assert.throws(
      () =>
        workflows["reviewed-task"].nodes.implement.validate(
          { ready: true, candidate },
          { input: { cwd: dir } },
        ),
      /Commit all task changes/,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("all model execution nodes explicitly avoid per-agent turn/time quotas", () => {
  for (const w of Object.values(workflows))
    for (const n of Object.values(w.nodes))
      if (n.nodeType === "agent") assert.equal(n.timeoutMs, null);
});
test("approval saves exact packets and does not call a launch node", () => {
  assert.equal(
    workflows["prepare-landing"].edges.some((e) => e.from === "approved"),
    false,
  );
  const launch = workflows["launch-landing"];
  assert.equal(launch.nodes.launch.allowedTools.includes("rexpi_start_pi"), true);
  assert.match(
    launch.nodes.launch.prompt({
      outputs: {
        prepare_workspaces: {
          jobs: [],
          evalInput: { cwd: "/tmp", model: { provider: "x", model: "y" } },
          evalWorkflow: "/tmp/eval",
        },
      },
    }),
    /returnResults:false/,
  );
});
test("verification is fail-loud on zero tests, failure or missing summaries", () => {
  assert.equal(passingTests("test result: ok. 3 passed; 0 failed;"), true);
  assert.equal(passingTests("test result: ok. 0 passed; 0 failed;"), false);
  assert.equal(passingTests("test result: FAILED. 3 passed; 1 failed;"), false);
  assert.equal(passingTests("compiled successfully"), false);
  assert.equal(
    passingTests("test result: ok. 3 passed; 0 failed;\ntest result: FAILED. 0 passed; 1 failed;"),
    false,
  );
});
