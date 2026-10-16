#!/usr/bin/env node
// Stand-in until R publishes pi-acp-patched/test/tools/fidelity.mjs.
// facts.json (synthetic or R's) vs a pager .txt capture.
// A fact counts as shown when its string occurs in the capture.
// Ratio = shown / expected. Prints missing facts.

import fs from "node:fs";

const factsPath = process.argv[2];
const capturePath = process.argv[3];
if (!factsPath || !capturePath) {
  process.stderr.write("usage: codemode-fidelity.mjs <facts.json> <capture.txt>\n");
  process.exit(2);
}

const facts = JSON.parse(fs.readFileSync(factsPath, "utf8"));
const capture = fs.readFileSync(capturePath, "utf8");

const expected = [];

function add(kind, value) {
  if (value === undefined || value === null) return;
  const text = String(value);
  if (!text.trim()) return;
  expected.push({ kind, text });
}

for (const line of String(facts.script ?? "").split("\n")) add("script", line.trim());
for (const line of String(facts.output ?? "").split("\n")) add("output", line.trim());
for (const tool of facts.tools ?? []) {
  add("tool-id", tool.id);
  add("tool-name", tool.name);
  add("tool-status", tool.status);
  add("parent-id", tool.parent);
}
for (const call of facts.model_calls ?? []) {
  add("model-name", call.name === "none" || call.name === "roll-up" ? null : call.name);
  add("model-cost", call.cost);
}

const need = new Map();
for (const fact of expected) {
  const key = `${fact.kind}\0${fact.text}`;
  const row = need.get(key) ?? { ...fact, count: 0 };
  row.count += 1;
  need.set(key, row);
}

const missing = [];
let shown = 0;
for (const fact of need.values()) {
  const found = capture.split(fact.text).length - 1;
  const hit = Math.min(found, fact.count);
  shown += hit;
  if (hit < fact.count) {
    missing.push({ kind: fact.kind, text: fact.text, need: fact.count, found });
  }
}

const ratio = expected.length === 0 ? 1 : shown / expected.length;
const out = {
  shown,
  expected: expected.length,
  ratio: Number(ratio.toFixed(3)),
  missing,
};
process.stdout.write(JSON.stringify(out, null, 2) + "\n");
process.exit(ratio === 1 ? 0 : 1);
