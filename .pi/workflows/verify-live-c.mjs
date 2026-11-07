import { spawn } from "node:child_process";
import { closeSync, createReadStream, existsSync, mkdirSync, mkdtempSync, openSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { createInterface } from "node:readline";

const [cwd, branch, cRevision, runId] = process.argv.slice(2);
const env = {
  ...process.env,
  PATH: `${join(homedir(), ".cargo/bin")}${delimiter}${process.env.PATH ?? ""}`,
  CARGO_TARGET_DIR: join(cwd, "target"),
  CARGO_TERM_COLOR: "never",
};
const receipt = { ok: false, revision: null, logDirectory: null, checks: [], anomalies: [] };

async function run(id, command, args, tests = false) {
  const logPath = join(receipt.logDirectory, `${id}.log`);
  const fd = openSync(logPath, "w", 0o600);
  const started = Date.now();
  let outcome;
  try {
    outcome = await new Promise((done) => {
      const child = spawn(command, args, { cwd, env, stdio: ["ignore", fd, fd] });
      child.once("error", (error) => done({ exitCode: null, signal: null, error: error.message }));
      child.once("close", (exitCode, signal) => done({ exitCode, signal }));
    });
  } finally {
    closeSync(fd);
  }
  // Scan logs locally; only counts and failure excerpts enter workflow output.
  let passed = 0, failed = 0, summaries = 0, warnings = 0;
  let tail = "", diagnostics = "", following = 0;
  for await (const line of createInterface({ input: createReadStream(logPath), crlfDelay: Infinity })) {
    tail = `${tail}${line}\n`.slice(-2_000);
    if (/^warning(?::|\[)/.test(line)) warnings++;
    const summary = line.match(/^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed/);
    if (summary) { summaries++; passed += Number(summary[1]); failed += Number(summary[2]); }
    if (/^error(?:\[|:)|FAILED|panicked at|^failures:/.test(line)) following = 8;
    if (following-- > 0 && diagnostics.length < 2_000) diagnostics = `${diagnostics}${line}\n`.slice(0, 2_000);
  }
  const problem = outcome.error ?? (outcome.exitCode !== 0
    ? `Command ${outcome.signal ? `terminated by ${outcome.signal}` : `exited ${outcome.exitCode}`}`
    : tests && (!summaries || !passed || failed) ? "Tests had missing/empty/failing summaries" : null);
  const check = {
    id, ok: problem === null, exitCode: outcome.exitCode, signal: outcome.signal,
    durationMs: Date.now() - started, logPath, warnings,
    ...(tests ? { passed, failed, summaries } : {}),
  };
  receipt.checks.push(check);
  if (problem) receipt.anomalies.push({
    check: id, problem, logPath,
    excerpt: `${diagnostics}${diagnostics ? "\nLast log lines:\n" : ""}${tail}`,
  });
  return check.ok;
}

try {
  if (!cwd || !branch || !cRevision || !runId) throw new Error("Expected repository, branch, C revision and workflow run ID.");
  // Git metadata keeps complete logs outside the tracked working tree.
  const { execFileSync } = await import("node:child_process");
  const git = (...args) => execFileSync("git", args, { cwd, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  const logRoot = resolve(cwd, git("rev-parse", "--git-path", "workflow-logs"));
  mkdirSync(logRoot, { recursive: true, mode: 0o700 });
  receipt.logDirectory = mkdtempSync(join(logRoot, `${runId.replace(/[^a-zA-Z0-9_-]/g, "_")}-`));
  process.stderr.write(`Verification logs: ${receipt.logDirectory}\n`);
  const preflight = [
    "set -euo pipefail",
    'test "$(git branch --show-current)" = "$1"',
    "git diff --quiet",
    "git diff --cached --quiet",
    'git merge-base --is-ancestor "$2" HEAD',
  ].join("\n");
  if (await run("preflight", "bash", ["-c", preflight, "preflight", branch, cRevision])) {
    receipt.revision = git("rev-parse", "HEAD");
    const checks = [
      ["build", ["build", "--release", "-p", "xai-grok-pager-bin"], false],
      ["traces-tests", ["test", "--release", "-p", "xai-grok-traces", "--lib"], true],
      ["modal-tests", ["test", "--release", "-p", "xai-grok-pager", "--lib", "traces_modal::"], true],
    ];
    if (existsSync(join(cwd, "crates/codegen/xai-grok-pager/src/acp/live_feed.rs"))) {
      checks.push(["live-feed-tests", ["test", "--release", "-p", "xai-grok-pager", "--lib", "acp::live_feed::tests::"], true]);
    }
    let passed = true;
    for (const [id, args, tests] of checks) {
      if (!await run(id, "cargo", args, tests)) { passed = false; break; }
    }
    if (passed) {
      receipt.ok = await run("postflight", "bash", ["-c", [
        preflight,
        'test "$(git rev-parse HEAD)" = "$3"',
      ].join("\n"), "postflight", branch, cRevision, receipt.revision]);
    }
  }
} catch (error) {
  receipt.anomalies.push({ check: "verification-runner", problem: error.message, logPath: receipt.logDirectory });
}
// Expected check failures are data, not transport failures. The graph routes ok=false.
process.stdout.write(`${JSON.stringify(receipt)}\n`);
