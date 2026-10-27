// Plans one spec slice into step briefs (scout), then reviews each brief against the repo (reviewer). args: { slice, sliceText, verify }
const scout = await runs.run("plan", {
  agent: "opus-proposal-scout",
  task: `Slice: ${args.slice}
Write briefs to .pi/briefs/${args.slice}/NN.md and .pi/briefs/${args.slice}/index.md.
Verify command (the acceptance line of each brief): ${args.verify}

Slice text:
${args.sliceText}

End your reply with one line: BRIEFS: <path>, <path>, ... (step briefs in order, not index.md).`,
});
const line = (scout.output.match(/^BRIEFS:\s*(.+)$/m) ?? [])[1] ?? "";
const paths = line.split(",").map((p) => p.trim()).filter(Boolean);
if (!paths.length) return { ok: false, steps: [], reason: "scout listed no briefs" };
const steps = [];
for (const path of paths) {
  const r = await runs.run("review " + path, {
    agent: "opus-proposal-reviewer",
    reads: [path],
    task: `MODE: BRIEF REVIEW\nBrief: ${path}\n\nSlice text:\n${args.sliceText}`,
  });
  steps.push({ path, verdict: (r.output.match(/Brief verdict:\s*(OK|BLOCK)/) ?? [])[1] ?? "missing" });
}
return { ok: steps.every((s) => s.verdict === "OK"), steps };
