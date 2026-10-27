// One step: worker (no read tools) -> gate -> one fix round on red -> build review. args: { brief, verify, timeoutMs, sliceText }
if (args.brief.length > 8000) return { ok: false, stage: "brief", reason: "step too big: split" };
const gate = { command: "sh .pi/opus-proposal-gate.sh " + args.verify, output: "json", timeoutMs: args.timeoutMs ?? 600000 };
const blocked = (o) => (o.match(/^BLOCKED:\s*(.*)$/m) ?? [])[1];
let run = await runs.run("implement", { agent: "opus-proposal-worker", task: args.brief, gate });
let reason = blocked(run.output);
if (reason) return { ok: false, stage: "implement", reason };
let g = run.structuredOutput;
if (!g?.ok) {
  run = await runs.run("fix", {
    agent: "opus-proposal-worker",
    task: `${args.brief}\n\nYour edits are already applied. Current diff:\n${g?.diff ?? "(none)"}\n\nThe gate failed:\n${g?.output ?? run.output}\n\nFix it.`,
    gate,
  });
  reason = blocked(run.output);
  if (reason) return { ok: false, stage: "fix", reason };
  g = run.structuredOutput;
  if (!g?.ok) return { ok: false, stage: "fix", gate: g?.output ?? run.output };
}
const review = await runs.run("review", {
  agent: "opus-proposal-reviewer",
  task: `MODE: BUILD REVIEW\n\nSlice:\n${args.sliceText}\n\nBrief:\n${args.brief}\n\nDiff:\n${g.diff}\n\nGate (passed):\n${g.output.slice(-1500)}`,
});
const verdict = (review.output.match(/Merge verdict:\s*(BLOCK|OK with notes|OK)/) ?? [])[1] ?? "missing";
return { ok: verdict === "OK" || verdict === "OK with notes", verdict, review: review.output.slice(-3000) };
