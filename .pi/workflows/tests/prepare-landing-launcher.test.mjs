import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { tmpdir } from "node:os";
const script = new URL("../prepare-landing-pi.sh", import.meta.url).pathname;
test("packet launcher uses parent identity, intended Rex session, and only preparation", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "landing-launcher-"));
  try {
    const bin = path.join(dir, "bin"),
      out = path.join(dir, "args.json");
    mkdirSync(bin);
    writeFileSync(path.join(bin, "pi"), "#!/bin/sh\nexit 99\n", { mode: 0o755 });
    writeFileSync(
      path.join(bin, "rex"),
      `#!${process.execPath}\nrequire('node:fs').writeFileSync(process.env.ARGS_OUT,JSON.stringify(process.argv.slice(2)));\n`,
      { mode: 0o755 },
    );
    const env = {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      PI_PROVIDER: "fixture-provider",
      PI_MODEL: "fixture-model",
      REX_SESSION: "session:fixture",
      ARGS_OUT: out,
    };
    const r = spawnSync("bash", [script], { env, encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    const args = JSON.parse(readFileSync(out, "utf8"));
    assert.equal(args[args.indexOf("-s") + 1], "session:fixture");
    assert.equal(args[args.indexOf("--provider") + 1], "fixture-provider");
    assert.equal(args[args.indexOf("--model") + 1], "fixture-model");
    assert.match(args.at(-1), /prepare-landing.workflow.ts/);
    assert.match(args.at(-1), /Do not launch builders/);
    delete env.PI_PROVIDER;
    assert.notEqual(spawnSync("bash", [script], { env, encoding: "utf8" }).status, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
