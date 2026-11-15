// Actual engine + fake Rex/cargo executables. No server dispatch or Cargo compilation.
import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { tmpdir } from "node:os";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
const root = process.env.PI_WORKFLOWS_ROOT;
const { loadWorkflowFile } = await import(path.join(root, "src/workflows/loader.ts"));
const { WorkflowEngine } = await import(path.join(root, "src/workflows/engine.ts"));
const workflow = await loadWorkflowFile(
  new URL("../lib/visible-command.workflow.ts", import.meta.url).pathname,
);
for (const mode of ["pass", "failed", "missing"])
  test(`runtime command actual engine: ${mode}`, async () => {
    const dir = mkdtempSync(path.join(tmpdir(), "landing-visible-")),
      cwd = path.join(dir, "repo"),
      home = path.join(dir, "home");
    const saved = {
      HOME: process.env.HOME,
      PATH: process.env.PATH,
      FIXTURE_MODE: process.env.FIXTURE_MODE,
    };
    try {
      mkdirSync(cwd);
      mkdirSync(path.join(home, ".local/bin"), { recursive: true });
      mkdirSync(path.join(home, ".cargo/bin"), { recursive: true });
      const git = (...args) => execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
      git("init", "-q");
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
      writeFileSync(
        path.join(home, ".local/bin/rex"),
        `#!${process.execPath}\nconst {spawnSync}=require('node:child_process');const args=process.argv.slice(2);if(process.env.FIXTURE_MODE==='missing')process.exit(2);const start=args.indexOf('--')+1;const r=spawnSync(args[start],args.slice(start+1),{stdio:'inherit',env:process.env});process.exit(r.status??1);`,
        { mode: 0o755 },
      );
      writeFileSync(path.join(home, ".local/bin/pgrep"), "#!/bin/sh\nexit 1\n", { mode: 0o755 });
      writeFileSync(
        path.join(home, ".cargo/bin/cargo"),
        `#!${process.execPath}\nconsole.log(process.env.FIXTURE_MODE==='failed'?'test result: FAILED. 0 passed; 1 failed;':'test result: ok. 1 passed; 0 failed;');process.exitCode=process.env.FIXTURE_MODE==='failed'?1:0;`,
        { mode: 0o755 },
      );
      const candidateFile = path.join(dir, "candidate.json");
      writeFileSync(
        candidateFile,
        JSON.stringify({
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
        }),
      );
      process.env.HOME = home;
      process.env.PATH = `${home}/.local/bin:${saved.PATH}`;
      process.env.FIXTURE_MODE = mode;
      const engine = new WorkflowEngine({
        databasePath: path.join(dir, "state.sqlite"),
        executor: {
          enforcesToolAllowlist: true,
          async runAgentStep() {
            throw Error("No agent turn authorized");
          },
        },
      });
      const { state } = await engine.run(workflow, {
        cwd,
        candidateFile,
        dir,
        rexSession: "session:fixture",
        mode: "tests",
      });
      assert.equal(state.status, "completed");
      assert.equal(state.finalOutput.ok, mode === "pass");
      if (mode === "pass") {
        assert.equal(state.finalOutput.checks[0].code, 0);
        assert.ok(state.finalOutput.nonce);
      }
    } finally {
      for (const [k, v] of Object.entries(saved)) {
        if (v === undefined) delete process.env[k];
        else process.env[k] = v;
      }
      rmSync(dir, { recursive: true, force: true });
    }
  });
