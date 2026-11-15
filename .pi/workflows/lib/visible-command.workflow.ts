import { randomUUID } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import path from "node:path";
import { action, compute, defineWorkflow, manualEffect, shell } from "@osolmaz/pi-workflows";
import { digest, parseCandidate } from "./landing-contracts.mjs";
const runner = fileURLToPath(new URL("./verify-landing.mjs", import.meta.url));

// Commands are authored/runtime-owned. No model turn can fabricate their outcome.
export default defineWorkflow({
  source: import.meta.url,
  name: "visible-command",
  input: (raw) => {
    const v = raw as any;
    if (
      !v ||
      !path.isAbsolute(v.cwd) ||
      !path.isAbsolute(v.candidateFile) ||
      !path.isAbsolute(v.dir) ||
      typeof v.rexSession !== "string" ||
      !v.rexSession.startsWith("session:") ||
      !["tests", "install"].includes(v.mode)
    )
      throw new Error(
        "Need checkout, candidate, artifact directory, explicit Rex session and mode",
      );
    return v;
  },
  startAt: "prepare",
  maxSteps: 5,
  nodes: {
    prepare: action({
      effect: manualEffect("landing.command-ticket"),
      run: ({ input }) => {
        const v = input as any,
          nonce = randomUUID();
        const candidate = parseCandidate(JSON.parse(readFileSync(v.candidateFile, "utf8")));
        const ticketFile = path.join(v.dir, `${v.mode}-${nonce}-ticket.json`),
          receiptFile = path.join(v.dir, `${v.mode}-${nonce}-receipt.json`);
        writeFileSync(
          ticketFile,
          JSON.stringify({ nonce, candidateDigest: digest(candidate), receiptFile, mode: v.mode }),
          { flag: "wx", mode: 0o600 },
        );
        return {
          nonce,
          ticketFile,
          receiptFile,
          candidateDigest: digest(candidate),
          revision: candidate.revision,
        };
      },
    }),
    command: shell({
      effect: manualEffect("landing.visible-command"),
      timeoutMs: 46_860_000,
      exec: ({ input, outputs }) => {
        const v = input as any,
          p = outputs.prepare as any;
        return {
          command: "rex",
          cwd: v.cwd,
          timeoutMs: 46_800_000,
          env: {
            PATH: `${homedir()}/.local/bin:${homedir()}/.cargo/bin:/opt/homebrew/bin:${process.env.PATH ?? ""}`,
          },
          allowNonZeroExit: true,
          args: [
            "run",
            "-s",
            v.rexSession,
            "-c",
            v.cwd,
            "--label",
            `landing-${v.mode}`,
            "--focus=false",
            "--shell=none",
            "--wait",
            "--json",
            "--",
            process.execPath,
            runner,
            v.cwd,
            v.candidateFile,
            p.receiptFile,
            v.mode,
            p.ticketFile,
          ],
        };
      },
      parse: (result) => ({
        exitCode: result.exitCode,
        stdout: result.stdout,
        stderr: result.stderr,
      }),
    }),
    receipt: action({
      effect: manualEffect("landing.command-receipt"),
      run: ({ input, outputs }) => {
        const v = input as any,
          p = outputs.prepare as any,
          cmd = outputs.command as any;
        if (!existsSync(p.receiptFile))
          return {
            ok: false,
            reason: "No command receipt; inspect the recorded Rex pane before any retry",
            launch: cmd,
          };
        const r = JSON.parse(readFileSync(p.receiptFile, "utf8"));
        if (
          r.nonce !== p.nonce ||
          r.candidateDigest !== p.candidateDigest ||
          r.revision !== p.revision ||
          r.mode !== v.mode
        )
          throw new Error("Command receipt binding mismatch");
        const candidate = parseCandidate(JSON.parse(readFileSync(v.candidateFile, "utf8")));
        const expected =
          v.mode === "tests" ? candidate.checks.map((c: any) => c.id) : ["install-fork"];
        if (
          r.ok &&
          (cmd.exitCode !== 0 ||
            digest(r.checks.map((c: any) => c.id)) !== digest(expected) ||
            !r.checks.every((c: any) => c.ok && c.code === 0))
        )
          throw new Error("Incomplete command receipt cannot pass");
        return { ...r, file: p.receiptFile, launch: cmd };
      },
    }),
    failed: compute({
      run: ({ results, outputs }) => ({
        ok: false,
        reason:
          "Visible command failed or was interrupted; inspect its exact Rex process and receipt before recovery",
        results,
        ticket: outputs.prepare,
      }),
    }),
  },
  exits: { finished: { from: "receipt" }, failed: { from: "failed" } },
  edges: [
    { from: "prepare", to: "command" },
    {
      from: "command",
      switch: {
        on: "$result.outcome",
        cases: { ok: "receipt", failed: "failed", timed_out: "failed" },
      },
    },
  ],
});
