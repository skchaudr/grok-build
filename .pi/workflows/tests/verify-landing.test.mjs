import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { digest } from "../lib/landing-contracts.mjs";
const script = new URL("../lib/verify-landing.mjs", import.meta.url).pathname;
for (const mode of ["pass", "zero", "failed", "timeout", "dirty"])
  test(`serial runner ${mode}: saved receipt, fail-loud checks`, () => {
    const dir = mkdtempSync(path.join(tmpdir(), "landing-runner-"));
    try {
      const cwd = path.join(dir, "repo"),
        bin = path.join(dir, "bin");
      mkdirSync(cwd);
      mkdirSync(bin);
      const git = (...a) => execFileSync("git", a, { cwd, encoding: "utf8" }).trim();
      git("init", "-q");
      writeFileSync(path.join(cwd, "fixture"), "initial");
      git("add", "fixture");
      git(
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-qm",
        "fixture",
      );
      const revision = git("rev-parse", "HEAD");
      writeFileSync(path.join(bin, "pgrep"), "#!/bin/sh\nexit 1\n", { mode: 0o755 });
      writeFileSync(
        path.join(bin, "cargo"),
        `#!${process.execPath}\nconst fs=require('node:fs');const mode=process.env.FIXTURE_MODE;if(mode==='timeout'){setTimeout(()=>{},10000);}else{if(mode==='dirty')fs.writeFileSync('fixture','changed');console.log(mode==='zero'?'test result: ok. 0 passed; 0 failed;':mode==='failed'?'test result: FAILED. 1 passed; 1 failed;':'test result: ok. 2 passed; 0 failed;');process.exitCode=mode==='failed'?101:0;}\n`,
        { mode: 0o755 },
      );
      const candidateFile = path.join(dir, "candidate.json"),
        receiptFile = path.join(dir, "receipt.json");
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
              timeoutMs: mode === "timeout" ? 100 : 5000,
            },
          ],
          smoke: ["Observe goal"],
        }),
      );
      const ticketFile = path.join(dir, "ticket.json");
      const candidate = JSON.parse(readFileSync(candidateFile, "utf8"));
      writeFileSync(
        ticketFile,
        JSON.stringify({
          nonce: "test-nonce",
          candidateDigest: digest(candidate),
          receiptFile,
          mode: "tests",
        }),
      );
      const r = spawnSync(
        process.execPath,
        [script, cwd, candidateFile, receiptFile, "tests", ticketFile],
        {
          env: {
            ...process.env,
            HOME: dir,
            PATH: `${bin}:${process.env.PATH}`,
            FIXTURE_MODE: mode,
          },
          encoding: "utf8",
          timeout: 10000,
        },
      );
      const receipt = JSON.parse(readFileSync(receiptFile, "utf8"));
      assert.equal(receipt.ok, mode === "pass");
      assert.equal(r.status, mode === "pass" ? 0 : 1);
      assert.equal(receipt.revision, revision);
      assert.equal(receipt.nonce, "test-nonce");
      assert.equal(receipt.candidateDigest, digest(candidate));
      assert.equal(receipt.checks.length, 1);
      if (mode === "timeout") assert.equal(receipt.checks[0].timedOut, true);
      if (mode === "dirty") assert.equal(receipt.unchanged, false);
      assert.match(r.stdout, /=== goal:/);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
