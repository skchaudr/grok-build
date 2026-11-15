import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import {
  action,
  agent,
  choice,
  compute,
  defineHumanChoices,
  defineWorkflow,
  humanDecision,
  humanDecisionEdge,
  includeWorkflow,
  manualEffect,
  shell,
} from "@osolmaz/pi-workflows";
import independentReview from "./lib/independent-review.workflow.ts";
import visibleCommand from "./lib/visible-command.workflow.ts";
import {
  digest,
  parseCandidate,
  parsePackets,
  parseModel,
  parseReviewBound,
} from "./lib/landing-contracts.mjs";

const git = (cwd: string, ...args: string[]) =>
  execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
const deliveryChoices = defineHumanChoices({
  deliver: choice({ label: "Fast-forward main, push, build/install both binaries on Mini + Air" }),
  hold: choice({ label: "Keep verified candidate; do not deliver" }),
});
const q = (s: string) => "'" + s.replaceAll("'", "'\\''") + "'";

export default defineWorkflow({
  source: import.meta.url,
  name: "integrate-landing",
  title: "Synthesize → final goal audit → tests → delivery",
  input: (raw) => {
    const v = raw as any;
    if (!v || !path.isAbsolute(v.integrationInput))
      throw new Error("Need launch-landing integrationInput path");
    const b = JSON.parse(readFileSync(v.integrationInput, "utf8"));
    parsePackets(b.packets);
    parseModel(b.reviewer);
    parseReviewBound(b.reviewRounds);
    if (!path.isAbsolute(b.cwd) || !/^[a-f0-9]{40}$/.test(b.base) || !b.goal?.trim())
      throw new Error("Invalid integration manifest");
    if (typeof v.rexSession !== "string" || !v.rexSession.startsWith("session:"))
      throw new Error("Explicit intended rexSession required");
    return { ...b, rexSession: v.rexSession };
  },
  startAt: "collect",
  maxSteps: 40,
  includes: {
    run_tests: includeWorkflow(visibleCommand, {
      input: ({ input, outputs }) => ({
        cwd: (input as any).cwd,
        rexSession: (input as any).rexSession,
        candidateFile: (outputs.candidate as any).file,
        dir: (outputs.candidate as any).dir,
        mode: "tests",
      }),
    }),
    run_install: includeWorkflow(visibleCommand, {
      input: ({ input, outputs }) => ({
        cwd: (input as any).cwd,
        rexSession: (input as any).rexSession,
        candidateFile: (outputs.candidate as any).file,
        dir: (outputs.candidate as any).dir,
        mode: "install",
      }),
    }),
    final_review: includeWorkflow(independentReview, {
      input: ({ input, outputs }) => {
        const v = input as any,
          p = outputs.candidate as any;
        return {
          cwd: v.cwd,
          revision: p.revision,
          goal: v.goal,
          acceptance: v.packets.flatMap((p: any) => p.acceptance),
          context: [
            `Read the combined diff from ${v.base} to ${p.revision}. Verify feature interaction, not merely each branch in isolation.`,
            `Independent acceptance design: ${v.acceptanceFile}`,
          ],
          model: v.reviewer,
          previous: null,
          label: "final-goal-audit",
        };
      },
    }),
  },
  nodes: {
    collect: action({
      effect: manualEffect("landing.collect-handoffs"),
      run: ({ input, state }) => {
        const v = input as any;
        if (
          git(v.cwd, "rev-parse", "main") !== v.base ||
          git(v.cwd, "status", "--porcelain", "--", ".", ":!.worktrees/")
        )
          throw new Error("Main moved or tracked work is dirty; inspect before integration");
        const handoffs = v.resultFiles.map((f: string) => JSON.parse(readFileSync(f, "utf8")));
        if (handoffs.length !== 3 || new Set(handoffs.map((h: any) => h.task)).size !== 3)
          throw new Error("Need exactly three distinct handoffs");
        for (const p of v.packets) {
          const h = handoffs.find((h: any) => h.task === p.id);
          if (!h || h.status !== "ready" || h.packetDigest !== digest(p))
            throw new Error(
              `Task ${p.id} is missing, blocked, or no longer matches its packet; preserve its handoff and scope repair separately`,
            );
          if (
            h.review?.verdict !== "ready" ||
            h.review.revision !== h.candidate?.revision ||
            h.review.goalDigest !== digest({ goal: p.goal, acceptance: p.acceptance }) ||
            git(h.cwd, "rev-parse", "HEAD") !== h.candidate.revision
          )
            throw new Error(`Task ${p.id} has stale review evidence`);
        }
        const acceptance = JSON.parse(readFileSync(v.acceptanceFile, "utf8"));
        if (acceptance.status !== "designed" || acceptance.goal !== v.goal)
          throw new Error("Missing independent goal-driven acceptance design");
        const dir = path.join(v.cwd, ".pi", "lane", state.runId);
        mkdirSync(dir, { recursive: true });
        return { handoffs, acceptance, dir };
      },
    }),
    synthesize: agent({
      timeoutMs: null,
      allowedTools: ["read", "bash", "edit", "write", "codemode"],
      prompt: ({ input, outputs }) => {
        const v = input as any,
          c = outputs.collect as any;
        return [
          `You are the integration parent in ${v.cwd}. Overall goal: ${v.goal}. Approved packets: ${JSON.stringify(v.packets)}.`,
          `Reviewed incoming work: ${JSON.stringify(c.handoffs)}. Independent acceptance design: ${JSON.stringify(c.acceptance)}.`,
          `Create a task integration branch from pinned main ${v.base} in this checkout. Preserve unrelated files. Assemble the reviewed commits without dropping either side of overlapping state-handling changes.`,
          "Inspect incoming work against the goals, resolve actual overlaps, and add the minimal missing acceptance tests identified independently. Do not rewrite working features or expand scope.",
          "No Cargo, install, push, main merge, workflow edits, new reviewers or dispatch here. Commit explicit task paths. Do not cherry-pick a branch's main ancestry as new changes.",
          "When you believe the combined work is ready for the final independent goal audit, return {ready:true,candidate:{revision,summary,checks,smoke}}. Consolidate duplicate test commands and use scoped cargo test --release commands. Do not run them yet.",
          "If blocked, return {ready:false,reason:'precise issue and preserved work'}. A failed final audit will return evidence for separately scoped follow-up; do not predesign every recovery case.",
        ].join("\n");
      },
      expectedOutput:
        "{ready:true,candidate:{revision,summary,checks,smoke}} or {ready:false,reason:string}",
      validate: (raw, { input, outputs }) => {
        const v = raw as any;
        if (v?.ready === false && typeof v.reason === "string") return v;
        const candidate = parseCandidate(v?.candidate);
        const design = parseCandidate((outputs.collect as any).acceptance.design);
        const acceptanceChecks = design.checks;
        for (const required of design.smoke)
          if (!candidate.smoke.includes(required))
            throw new Error(`Independent smoke outcome omitted: ${required}`);
        for (const required of acceptanceChecks)
          if (!candidate.checks.some((check: any) => digest(check) === digest(required)))
            throw new Error(`Independent acceptance check omitted or changed: ${required.id}`);
        if (
          git((input as any).cwd, "rev-parse", "HEAD") !== candidate.revision ||
          git((input as any).cwd, "branch", "--show-current") === "main"
        )
          throw new Error("Need committed candidate on integration branch");
        if (git((input as any).cwd, "status", "--porcelain", "--", ".", ":!.worktrees/"))
          throw new Error("Commit all integration changes, including new tests");
        return { ready: true, candidate };
      },
    }),
    synthesis_route: compute({
      run: ({ outputs }) => ({ ready: (outputs.synthesize as any).ready }),
    }),
    candidate: action({
      effect: manualEffect("landing.candidate"),
      run: ({ outputs }) => {
        const p = parseCandidate((outputs.synthesize as any).candidate),
          dir = (outputs.collect as any).dir;
        const file = path.join(dir, "candidate.json");
        writeFileSync(file, JSON.stringify(p, null, 2), { flag: "wx" });
        return { ...p, file, dir };
      },
    }),
    tests: compute({ run: ({ outputs }) => (outputs.run_tests as any).output }),
    test_route: compute({ run: ({ outputs }) => ({ ok: (outputs.tests as any).ok === true }) }),
    deliver_gate: humanDecision({
      audience: "operator",
      choices: deliveryChoices,
      request: ({ outputs }) => ({
        title: "Reviewed and tested candidate: authorize delivery?",
        subject: {
          candidate: outputs.candidate,
          review: outputs.final_review,
          tests: outputs.tests,
        },
        presentation: {
          schema: "pi-workflows.decision-presentation.v1",
          summary:
            "Explicit delivery permission: fast-forward main, push main, run install-fork.sh once, then live verification.",
          blocks: [
            {
              kind: "preformatted",
              text: JSON.stringify(
                {
                  revision: (outputs.candidate as any).revision,
                  summary: (outputs.candidate as any).summary,
                  finalAudit: outputs.final_review,
                  tests: outputs.tests,
                },
                null,
                2,
              ),
            },
          ],
        },
      }),
    }),
    land: shell({
      effect: manualEffect("landing.land-main"),
      timeoutMs: 120_000,
      exec: ({ input, outputs }) => {
        const v = input as any,
          revision = (outputs.candidate as any).revision;
        return {
          command: "bash",
          cwd: v.cwd,
          timeoutMs: 120_000,
          args: [
            "-c",
            [
              "set -euo pipefail",
              `test "$(git rev-parse HEAD)" = ${q(revision)}`,
              `test "$(git rev-parse main)" = ${q(v.base)}`,
              "git diff --quiet",
              "git diff --cached --quiet",
              "git switch main",
              `git merge --ff-only ${q(revision)}`,
              "git push origin main",
            ].join("\n"),
          ],
        };
      },
    }),
    install: compute({ run: ({ outputs }) => (outputs.run_install as any).output }),
    install_route: compute({
      run: ({ outputs }) => ({ ok: (outputs.install as any).ok === true }),
    }),
    smoke: agent({
      timeoutMs: null,
      allowedTools: ["read", "bash"],
      prompt: ({ input, outputs }) =>
        `Verify the shipped user outcomes with real visible Rex screens on Mini and sab-air, not guesses from code. Goal ${(input as any).goal}; approved packets ${JSON.stringify((input as any).packets)}; minimal checks ${JSON.stringify((outputs.candidate as any).smoke)}. Record exactly what was observed and what remains unverified. Colour changes require the shipped-theme screen checks in AGENTS.md. Use bash only to inspect checksums/state and drive the existing product in visible Rex screens. Do not rebuild, reinstall, edit source, or run load tests on the Air. This is a trusted read/interaction contract, not a shell sandbox; no one-install security guarantee is claimed for arbitrary same-user shell actions. Return {ok:boolean,evidence:[strings],unverified:[strings]}; any unverified required outcome means ok:false.`,
      expectedOutput: "{ok:boolean,evidence:string[],unverified:string[]}",
      validate: (raw) => {
        const v = raw as any;
        if (
          typeof v?.ok !== "boolean" ||
          !Array.isArray(v.evidence) ||
          (v.ok && v.evidence.length === 0) ||
          !Array.isArray(v.unverified) ||
          (v.ok && v.unverified.length)
        )
          throw new Error("Need actual screen evidence and explicit unverified outcomes");
        return v;
      },
    }),
    finish: compute({
      run: ({ outputs }) => ({
        status: (outputs.smoke as any).ok ? "delivered-and-verified" : "delivered-needs-follow-up",
        revision: (outputs.candidate as any).revision,
        tests: outputs.tests,
        install: outputs.install,
        smoke: outputs.smoke,
      }),
    }),
    hold: compute({
      run: ({ outputs }) => ({
        status: "verified-not-delivered",
        candidate: outputs.candidate,
        tests: outputs.tests,
      }),
    }),
    blocked: compute({
      run: ({ outputs, results }) => ({
        status: "needs-scoped-follow-up",
        candidate: outputs.candidate ?? null,
        evidence: {
          synthesis: outputs.synthesize,
          finalReview: outputs.final_review,
          tests: outputs.tests,
          install: outputs.install,
          errors: Object.fromEntries(
            Object.entries(results)
              .filter(([, r]) => r.outcome !== "ok")
              .map(([id, r]) => [id, r.error]),
          ),
        },
        next: "Parent and operator inspect the concrete scope of this finding before selecting repair. Preserve accepted commits and do not start a generic retry workflow or second install.",
      }),
    }),
  },
  edges: [
    {
      from: "collect",
      switch: {
        on: "$result.outcome",
        cases: { ok: "synthesize", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "synthesize",
      switch: {
        on: "$result.outcome",
        cases: { ok: "synthesis_route", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "synthesis_route",
      switch: { on: "$.ready", cases: { true: "candidate", false: "blocked" } },
    },
    { from: "candidate", to: "final_review" },
    { from: "final_review.ready", to: "run_tests" },
    { from: "final_review.changes", to: "blocked" },
    { from: "final_review.blocked", to: "blocked" },
    { from: "run_tests.finished", to: "tests" },
    { from: "run_tests.failed", to: "blocked" },
    { from: "run_install.finished", to: "install" },
    { from: "run_install.failed", to: "blocked" },
    {
      from: "tests",
      switch: {
        on: "$result.outcome",
        cases: { ok: "test_route", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "test_route",
      switch: { on: "$.ok", cases: { true: "deliver_gate", false: "blocked" } },
    },
    humanDecisionEdge({
      from: "deliver_gate",
      choices: deliveryChoices,
      cases: { deliver: "land", hold: "hold" },
    }),
    {
      from: "land",
      switch: {
        on: "$result.outcome",
        cases: { ok: "run_install", failed: "blocked", timed_out: "blocked" },
      },
    },
    {
      from: "install",
      switch: {
        on: "$result.outcome",
        cases: { ok: "install_route", failed: "blocked", timed_out: "blocked" },
      },
    },
    { from: "install_route", switch: { on: "$.ok", cases: { true: "smoke", false: "blocked" } } },
    { from: "smoke", to: "finish" },
  ],
});
