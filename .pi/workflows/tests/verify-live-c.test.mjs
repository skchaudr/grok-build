import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(new URL("../verify-live-c.mjs", import.meta.url));
const branch = "integrate/traces-live-c";

function fixture(mode, liveFeed = false) {
  const root = mkdtempSync(join(tmpdir(), "verify-live-c-test-"));
  const repo = join(root, "repo"), home = join(root, "home");
  mkdirSync(repo); mkdirSync(join(home, ".cargo/bin"), { recursive: true });
  const git = (...args) => execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init", "-q"); git("config", "user.name", "Workflow Test"); git("config", "user.email", "test@example.invalid");
  writeFileSync(join(repo, "fixture.txt"), "synthetic fixture\n");
  git("add", "fixture.txt");
  if (liveFeed) {
    const path = join(repo, "crates/codegen/xai-grok-pager/src/acp/live_feed.rs");
    mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, "// fixture\n");
    git("add", "crates/codegen/xai-grok-pager/src/acp/live_feed.rs");
  }
  git("commit", "-qm", "synthetic verification fixture");
  const revision = git("rev-parse", "HEAD");
  git("switch", "-qc", branch);
  writeFileSync(join(home, ".cargo/bin/cargo"), `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
const mode = process.env.FAKE_VERIFY_MODE;
if (args[0] === 'build') {
  process.stdout.write('BUILD_VERBOSE_SENTINEL\\n' + 'x'.repeat(1_100_000) + '\\n');
  process.stderr.write('STDERR_LOG_SENTINEL\\n');
  if (mode === 'build-fail') { process.stderr.write('error: synthetic build failure\\n'); process.exit(2); }
  if (mode === 'dirty') fs.writeFileSync('fixture.txt', 'changed during build\\n');
} else {
  const live = args.includes('acp::live_feed::tests::');
  if (mode === 'test-fail') { process.stdout.write('test result: FAILED. 2 passed; 1 failed\\n'); process.stderr.write('error: synthetic test failure\\n'); process.exit(101); }
  if (mode === 'empty-tests' || (mode === 'empty-live' && live)) process.stdout.write('test result: ok. 0 passed; 0 failed\\n');
  else if (mode === 'missing-summary') process.stdout.write('finished without a test summary\\n');
  else process.stdout.write('test result: ok. 3 passed; 0 failed\\n');
}
`, { mode: 0o700 });
  const result = spawnSync(process.execPath, [script, repo, branch, revision, "test-run"], {
    env: { ...process.env, HOME: home, FAKE_VERIFY_MODE: mode }, encoding: "utf8",
  });
  return { root, revision, result, receipt: result.stdout ? JSON.parse(result.stdout) : null };
}

function check(mode, liveFeed, inspect) {
  const f = fixture(mode, liveFeed);
  try {
    assert.equal(f.result.status, 0, f.result.stderr);
    assert.ok(f.result.stdout.length < 10_000, "Only a compact receipt should be captured");
    assert.ok(!f.result.stdout.includes("BUILD_VERBOSE_SENTINEL"));
    assert.ok(!f.result.stderr.includes("STDERR_LOG_SENTINEL"));
    assert.ok(existsSync(f.receipt.logDirectory));
    for (const c of f.receipt.checks) assert.ok(existsSync(c.logPath));
    inspect(f);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
}

test("complete output over one million characters stays on disk; successful receipt stays small", () => {
  check("pass", true, ({ receipt, revision }) => {
    assert.equal(receipt.ok, true); assert.equal(receipt.revision, revision);
    assert.deepEqual(receipt.anomalies, []);
    assert.ok(receipt.checks.some(c => c.id === "live-feed-tests"));
    const build = receipt.checks.find(c => c.id === "build");
    assert.ok(statSync(build.logPath).size > 1_100_000);
    assert.match(readFileSync(build.logPath, "utf8"), /STDERR_LOG_SENTINEL/);
    assert.ok(receipt.checks.filter(c => c.id.endsWith("tests")).every(c => c.passed === 3));
  });
});

test("build failure reports only the anomaly and saved log path", () => {
  check("build-fail", false, ({ receipt }) => {
    assert.equal(receipt.ok, false);
    assert.equal(receipt.anomalies[0].check, "build");
    assert.match(receipt.anomalies[0].excerpt, /synthetic build failure/);
    assert.ok(receipt.anomalies[0].excerpt.length <= 4_022);
  });
});

for (const mode of ["test-fail", "empty-tests", "missing-summary", "empty-live", "dirty"]) {
  test(`${mode} produces a failed receipt with inspectable logs`, () => {
    check(mode, mode === "empty-live", ({ receipt }) => {
      assert.equal(receipt.ok, false); assert.ok(receipt.anomalies.length);
      assert.ok(receipt.anomalies.every(a => existsSync(a.logPath)));
    });
  });
}
