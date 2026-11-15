// Runtime contract validators for landing workflows (packet -> candidate ->
// review -> route). Explicit validators, no TypeScript dependencies.

import { createHash } from "node:crypto";

const PACKET_ID = new Set(["f3", "cass", "claude-settings"]);
const PACKET_KEYS = [
  "id",
  "goal",
  "context",
  "files",
  "constraints",
  "acceptance",
  "startingRef",
  "builder",
];
const BUILDER_KEYS = ["provider", "model", "thinking"];
const SEVERITY = new Set(["P0", "P1", "P2"]);
const VERDICT = new Set(["ready", "changes", "blocked"]);
export const APPROVED_CRATES = new Set(["xai-grok-pager", "xai-grok-pager-bin", "xai-grok-traces"]);
export const MAX_CHECKS = 12;
export const MAX_TIMEOUT_MS = 3_600_000;

const HEX40 = /^[0-9a-f]{40}$/;
const HEX64 = /^[0-9a-f]{64}$/;

export class ContractError extends Error {}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function fail(msg) {
  throw new ContractError(msg);
}

function exactKeys(value, keys, what) {
  if (!isPlainObject(value)) fail(`${what} must be an object`);
  const have = Object.keys(value);
  const missing = keys.filter((k) => !have.includes(k));
  if (missing.length > 0) fail(`${what} missing keys: ${missing.join(", ")}`);
  const unknown = have.filter((k) => !keys.includes(k));
  if (unknown.length > 0) fail(`${what} unknown keys: ${unknown.join(", ")}`);
}

function nonEmptyString(value, what) {
  if (typeof value !== "string" || value.trim().length === 0)
    fail(`${what} must be a nonempty string`);
  return value;
}

function nonEmptyStringArray(value, what) {
  if (!Array.isArray(value)) fail(`${what} must be an array`);
  if (value.length === 0) fail(`${what} must be nonempty`);
  return value.map((item, i) => nonEmptyString(item, `${what}[${i}]`));
}

function hex(value, length, what) {
  if (typeof value !== "string" || !(length === 40 ? HEX40 : HEX64).test(value)) {
    fail(`${what} must be ${length} lowercase hex chars`);
  }
  return value;
}

function checkRelPath(rel, what) {
  if (rel.startsWith("/") || /^[A-Za-z]:/.test(rel) || rel.includes("\\") || rel.includes("\0")) {
    fail(`${what} must be a repo-relative path: ${rel}`);
  }
  if (rel.split("/").includes("..")) fail(`${what} must not traverse parent directories: ${rel}`);
  if (rel.split("/").includes("")) fail(`${what} must not contain empty path segments: ${rel}`);
  for (const ch of ["*", "?", "[", "]", "{", "}", "|"]) {
    if (rel.includes(ch)) fail(`${what} must not contain wildcard/meta characters (${ch}): ${rel}`);
  }
  return rel;
}

function checkBuilder(builder) {
  exactKeys(builder, BUILDER_KEYS, "builder");
  nonEmptyString(builder.provider, "builder.provider");
  nonEmptyString(builder.model, "builder.model");
  if (builder.thinking !== "high") fail("builder.thinking must be 'high'");
  return { provider: builder.provider, model: builder.model, thinking: "high" };
}

export function parsePacket(value) {
  exactKeys(value, PACKET_KEYS, "packet");
  if (!PACKET_ID.has(value.id))
    fail(`packet.id must be one of f3|cass|claude-settings, got ${JSON.stringify(value.id)}`);
  const goal = nonEmptyString(value.goal, "packet.goal");
  const context = nonEmptyStringArray(value.context, "packet.context");
  const files = nonEmptyStringArray(value.files, "packet.files").map((f) =>
    checkRelPath(f, "packet.files"),
  );
  const constraints = nonEmptyStringArray(value.constraints, "packet.constraints");
  const acceptance = nonEmptyStringArray(value.acceptance, "packet.acceptance");
  const startingRef = nonEmptyString(value.startingRef, "packet.startingRef");
  return {
    id: value.id,
    goal,
    context,
    files,
    constraints,
    acceptance,
    startingRef,
    builder: checkBuilder(value.builder),
  };
}

export function parsePackets(value) {
  if (!Array.isArray(value)) fail("packets must be an array");
  if (value.length !== 3) fail(`packets must contain exactly 3 packets, got ${value.length}`);
  const parsed = value.map((p) => parsePacket(p));
  const ids = parsed.map((p) => p.id);
  if (new Set(ids).size !== 3) fail(`packets must have three distinct ids, got ${ids.join(",")}`);
  return parsed;
}

// Canonical form: object keys recursively sorted, stable separators, no spaces.
function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (isPlainObject(value)) {
    const out = Object.create(null);
    for (const key of Object.keys(value).sort()) out[key] = canonicalize(value[key]);
    return out;
  }
  return value;
}

export function digest(value) {
  const json = JSON.stringify(canonicalize(value));
  if (json === undefined) fail("digest input must be JSON-serializable");
  return createHash("sha256").update(json).digest("hex");
}

function parseFinding(value, i) {
  exactKeys(value, ["id", "severity", "file", "message"], `finding[${i}]`);
  const id = nonEmptyString(value.id, `finding[${i}].id`);
  if (!SEVERITY.has(value.severity)) fail(`finding[${i}].severity must be P0|P1|P2`);
  const file = nonEmptyString(value.file, `finding[${i}].file`);
  const message = nonEmptyString(value.message, `finding[${i}].message`);
  return { id, severity: value.severity, file, message };
}

export function parseReview(value) {
  exactKeys(value, ["revision", "goalDigest", "verdict", "findings", "summary"], "review");
  const revision = hex(value.revision, 40, "review.revision");
  const goalDigest = hex(value.goalDigest, 64, "review.goalDigest");
  if (!VERDICT.has(value.verdict)) fail("review.verdict must be ready|changes|blocked");
  if (!Array.isArray(value.findings)) fail("review.findings must be an array");
  const findings = value.findings.map((f, i) => parseFinding(f, i));
  const summary = nonEmptyString(value.summary, "review.summary");
  if (value.verdict === "ready" && findings.length !== 0)
    fail("verdict 'ready' requires zero findings");
  if (value.verdict === "changes" && findings.length === 0)
    fail("verdict 'changes' requires at least one finding");
  return { revision, goalDigest, verdict: value.verdict, findings, summary };
}

// Cargo-test check validation. Deliberately not an exhaustive cargo-flag
// whitelist: require `test --release -p <approved crate>` plus a nonempty
// scoped filter (post `--` or positional), and reject harness-bypass tokens.
const BANNED_TOKENS = new Set([
  "--ignored",
  "--include-ignored",
  "--no-run",
  "--list",
  "--doc",
  "--all",
  "--workspace",
  "--all-targets",
  "--exclude",
  "--target-dir",
  "--manifest-path",
  "--config",
  "--profile",
]);
const SHELL_META = new Set([";", "$", "`", "|", "&", ">", "<", "\n", "\r", "\0", "("]);
const VALUE_FLAGS = new Set([
  "-p",
  "--package",
  "--test",
  "--bench",
  "--example",
  "--target",
  "--manifest-path",
  "--exclude",
  "--config",
  "--jobs",
  "--profile",
  "--color",
  "--format",
  "--message-format",
  "--target-dir",
  "--features",
  "--unstable-features",
  "--report-time",
  "--logfile",
  "--include-ignored",
]);

function checkArg(arg, what) {
  if (typeof arg !== "string" || arg.length === 0) fail(`${what} args must be nonempty strings`);
  for (const meta of SHELL_META) {
    if (arg.includes(meta))
      fail(`${what} args must not contain shell-like characters (${JSON.stringify(meta)}): ${arg}`);
  }
  return arg;
}

function parseCheck(value, i) {
  const what = `check[${i}]`;
  exactKeys(value, ["id", "command", "args", "timeoutMs"], what);
  nonEmptyString(value.id, `${what}.id`);
  if (value.command !== "cargo") fail(`${what}.command must be 'cargo'`);
  if (!Array.isArray(value.args) || value.args.length === 0)
    fail(`${what}.args must be a nonempty array`);
  const args = value.args.map((a, j) => checkArg(a, `${what}.args[${j}]`));
  if (args[0] !== "test") fail(`${what} must invoke 'cargo test ...'`);
  if (!args.includes("--release")) fail(`${what} must run with --release`);
  const crateIdx = args.indexOf("-p");
  const harnessIdx = args.indexOf("--");
  if (harnessIdx !== -1 && args.indexOf("--release") > harnessIdx)
    fail(`${what} --release must precede harness arguments`);
  if (
    crateIdx === -1 ||
    crateIdx === args.length - 1 ||
    (harnessIdx !== -1 && crateIdx > harnessIdx) ||
    !APPROVED_CRATES.has(args[crateIdx + 1])
  ) {
    fail(
      `${what} must scope to an approved crate with -p xai-grok-pager|xai-grok-pager-bin|xai-grok-traces`,
    );
  }
  for (const token of args) {
    if (BANNED_TOKENS.has(token)) fail(`${what} must not use banned token '${token}'`);
  }
  if (
    args.filter((a) => a === "-p" || a === "--package" || a.startsWith("--package=")).length !== 1
  )
    fail(`${what} requires exactly one package`);
  for (const a of args) {
    if (
      [
        "--target-dir",
        "--manifest-path",
        "--config",
        "--profile",
        "--exclude",
        "--test",
        "--bench",
        "--example",
      ].some((flag) => a.startsWith(flag + "="))
    )
      fail(`${what} requires explicit scoped arguments, not ${a}`);
  }
  if (args.includes("--")) {
    const harness = args.slice(args.indexOf("--") + 1);
    const harnessValues = new Set(["--test-threads", "--format", "--color", "--logfile", "--skip"]);
    if (!harness.some((a, index) => !a.startsWith("-") && !harnessValues.has(harness[index - 1])))
      fail(`${what} requires a nonempty scoped filter after '--'`);
  } else {
    let positional = 0;
    for (let j = 1; j < args.length; j++) {
      const prev = args[j - 1];
      if (!args[j].startsWith("-") && !(prev.startsWith("-") && VALUE_FLAGS.has(prev)))
        positional++;
    }
    if (positional === 0)
      fail(`${what} requires a nonempty scoped test filter (post '--' or positional)`);
  }
  const timeoutMs = value.timeoutMs;
  if (!Number.isInteger(timeoutMs) || timeoutMs <= 0)
    fail(`${what}.timeoutMs must be a positive integer`);
  if (timeoutMs > MAX_TIMEOUT_MS) fail(`${what}.timeoutMs must be <= ${MAX_TIMEOUT_MS} (1h)`);
  return { id: value.id, command: "cargo", args, timeoutMs };
}

export function parseCandidate(value) {
  exactKeys(value, ["revision", "summary", "checks", "smoke"], "candidate");
  const revision = hex(value.revision, 40, "candidate.revision");
  const summary = nonEmptyString(value.summary, "candidate.summary");
  if (!Array.isArray(value.checks)) fail("candidate.checks must be an array");
  if (value.checks.length === 0)
    fail("candidate.checks must be nonempty (goal-driven tests required)");
  if (value.checks.length > MAX_CHECKS)
    fail(`candidate.checks must have at most ${MAX_CHECKS} checks`);
  const checks = value.checks.map((c, i) => parseCheck(c, i));
  const seen = new Set();
  for (const c of checks) {
    if (seen.has(c.id)) fail(`candidate.checks ids must be distinct, duplicate '${c.id}'`);
    seen.add(c.id);
  }
  if (!Array.isArray(value.smoke)) fail("candidate.smoke must be an array");
  const smoke = value.smoke.map((s, i) => nonEmptyString(s, `candidate.smoke[${i}]`));
  return { revision, summary, checks, smoke };
}

export function parseReviewBound(value) {
  if (!Number.isInteger(value) || value < 1 || value > 5)
    fail("reviewBound must be an integer in 1..5");
  return value;
}

function intIn(value, min, max, what) {
  if (!Number.isInteger(value) || value < min || value > max)
    fail(`${what} must be an integer in ${min}..${max}`);
  return value;
}

export function reviewRoute(review, revision, goalDigest, round, maxRounds) {
  const parsed = parseReview(review);
  const rev =
    typeof revision === "string"
      ? hex(revision, 40, "revision")
      : fail("revision must be a 40-hex string");
  const gd =
    typeof goalDigest === "string"
      ? hex(goalDigest, 64, "goalDigest")
      : fail("goalDigest must be a 64-hex string");
  if (parsed.revision !== rev)
    fail(`review.revision ${parsed.revision} does not match current revision ${rev}`);
  if (parsed.goalDigest !== gd) fail(`review.goalDigest does not match packet goal digest ${gd}`);
  const r = intIn(round, 1, Number.MAX_SAFE_INTEGER, "round");
  const m = intIn(maxRounds, 1, Number.MAX_SAFE_INTEGER, "maxRounds");
  if (r > m) fail(`round ${r} exceeds maxRounds ${m}`);
  if (parsed.verdict === "ready") return "ready";
  if (parsed.verdict === "blocked") return "blocked";
  return r >= m ? "blocked" : "repair";
}

export function parseModel(value) {
  exactKeys(value, ["provider", "model", "thinking"], "model");
  const provider = nonEmptyString(value.provider, "model.provider");
  const model = nonEmptyString(value.model, "model.model");
  if (value.thinking !== "high") fail("model.thinking must be 'high'");
  return { provider, model, thinking: "high" };
}
