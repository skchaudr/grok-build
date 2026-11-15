// Runtime-owned serial check/install runner. Launch only in a visible Rex job.
import { spawn, execFileSync } from "node:child_process";
import { createWriteStream, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { digest, parseCandidate } from "./landing-contracts.mjs";

export function passingTests(text) {
  const results = [...text.matchAll(/test result: (ok|FAILED)\. (\d+) passed; (\d+) failed/g)];
  return (
    results.length > 0 &&
    results.every((m) => m[1] === "ok" && Number(m[3]) === 0) &&
    results.some((m) => Number(m[2]) > 0)
  );
}
export async function execute({ cwd, candidateFile, receiptFile, mode, ticketFile }) {
  if (!["tests", "install"].includes(mode)) throw new Error("mode must be tests or install");
  const candidate = parseCandidate(JSON.parse(readFileSync(candidateFile, "utf8")));
  const ticket = ticketFile ? JSON.parse(readFileSync(ticketFile, "utf8")) : null;
  if (
    ticket &&
    (ticket.candidateDigest !== digest(candidate) ||
      ticket.mode !== mode ||
      ticket.receiptFile !== receiptFile)
  )
    throw new Error("Command ticket mismatch");
  const git = (...args) => execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
  if (
    git("rev-parse", "HEAD") !== candidate.revision ||
    git("status", "--porcelain", "--", ".", ":!.worktrees/")
  )
    throw new Error("Candidate must be current clean tracked HEAD");
  // Do not compete for the machine-wide Cargo lock.
  try {
    execFileSync("pgrep", ["-x", "cargo"], { stdio: "pipe" });
    throw new Error("Another cargo is running; inspect before retry");
  } catch (error) {
    if (error.status !== 1) throw error;
  }
  const dir = path.dirname(receiptFile);
  mkdirSync(dir, { recursive: true });
  const checks = [];
  let interrupted = false;
  const commands =
    mode === "tests"
      ? candidate.checks
      : [
          {
            id: "install-fork",
            command: path.join(cwd, "scripts/install-fork.sh"),
            args: [],
            timeoutMs: 3_600_000,
          },
        ];
  for (const [index, check] of commands.entries()) {
    const logPath = path.join(dir, `${mode}-${ticket?.nonce ?? "standalone"}-${index}.log`);
    console.log(`\n=== ${check.id}: ${check.command} ${check.args.join(" ")} ===`);
    const stream = createWriteStream(logPath, { flags: "wx" });
    const env = {
      ...process.env,
      PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH ?? ""}`,
    };
    const child = spawn(check.command, check.args, {
      cwd,
      env,
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    });
    let timedOut = false;
    let killTimer;
    const terminate = () => {
      try {
        process.kill(-child.pid, "SIGTERM");
      } catch {}
      killTimer ??= setTimeout(() => {
        try {
          process.kill(-child.pid, "SIGKILL");
        } catch {}
      }, 5000);
    };
    const interrupt = () => {
      interrupted = true;
      terminate();
    };
    const timer = setTimeout(() => {
      timedOut = true;
      terminate();
    }, check.timeoutMs);
    process.once("SIGTERM", interrupt);
    process.once("SIGINT", interrupt);
    child.stdout.on("data", (b) => {
      process.stdout.write(b);
      stream.write(b);
    });
    child.stderr.on("data", (b) => {
      process.stderr.write(b);
      stream.write(b);
    });
    let spawnError;
    child.on("error", (error) => {
      spawnError = error.message;
    });
    const [code, signal] = await new Promise((resolve) =>
      child.once("close", (...args) => resolve(args)),
    );
    clearTimeout(timer);
    clearTimeout(killTimer);
    process.removeListener("SIGTERM", interrupt);
    process.removeListener("SIGINT", interrupt);
    await new Promise((resolve) => stream.end(resolve));
    const text = readFileSync(logPath, "utf8");
    const ok =
      !interrupted &&
      !spawnError &&
      !timedOut &&
      code === 0 &&
      (mode === "tests"
        ? passingTests(text)
        : /grok-fork .* matches here and on /.test(text) &&
          /grok-traces .* matches here and on /.test(text));
    checks.push({ id: check.id, code, signal, timedOut, spawnError, ok, logPath });
    if (!ok || interrupted) break;
  }
  const unchanged =
    git("rev-parse", "HEAD") === candidate.revision &&
    !git("status", "--porcelain", "--", ".", ":!.worktrees/");
  const receipt = {
    mode,
    nonce: ticket?.nonce ?? null,
    candidateDigest: digest(candidate),
    revision: candidate.revision,
    ok: unchanged && checks.length === commands.length && checks.every((c) => c.ok),
    unchanged,
    checks,
  };
  writeFileSync(receiptFile, JSON.stringify(receipt, null, 2) + "\n", { flag: "wx" });
  console.log(`\nReceipt: ${receiptFile}\n${JSON.stringify(receipt)}`);
  return receipt;
}
if (process.argv[1] && path.resolve(process.argv[1]) === new URL(import.meta.url).pathname) {
  const [cwd, candidateFile, receiptFile, mode, ticketFile] = process.argv.slice(2);
  execute({ cwd, candidateFile, receiptFile, mode, ticketFile })
    .then((r) => {
      process.exitCode = r.ok ? 0 : 1;
    })
    .catch((error) => {
      console.error(error);
      process.exitCode = 1;
    });
}
