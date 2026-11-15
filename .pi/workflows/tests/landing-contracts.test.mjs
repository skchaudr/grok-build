// Tests for .pi/workflows/lib/landing-contracts.mjs — run with:
//   node --test .pi/workflows/tests/landing-contracts.test.mjs

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import {
  APPROVED_CRATES,
  MAX_CHECKS,
  MAX_TIMEOUT_MS,
  digest,
  parseCandidate,
  parseModel,
  parsePacket,
  parsePackets,
  parseReview,
  parseReviewBound,
  reviewRoute,
} from "../lib/landing-contracts.mjs";

const REV = "a".repeat(40);
const REV2 = "b".repeat(40);
const GD = "c".repeat(64);
const GD2 = "d".repeat(64);

const VALID_PACKET = {
  id: "f3",
  goal: "Land the pager fix",
  context: ["crates/codegen/xai-grok-pager/src/lib.rs"],
  files: ["crates/codegen/xai-grok-pager/src/acp/live_feed.rs"],
  constraints: ["No new dependencies"],
  acceptance: ["cargo test --release -p xai-grok-pager passes"],
  startingRef: "origin/main",
  builder: { provider: "xai", model: "grok-4", thinking: "high" },
};

const packet = (overrides = {}) => {
  const out = structuredClone(VALID_PACKET);
  for (const [k, v] of Object.entries(overrides)) {
    if (v === null) delete out[k];
    else out[k] = v;
  }
  return out;
};

const review = (overrides = {}) => ({
  revision: REV,
  goalDigest: GD,
  verdict: "changes",
  findings: [{ id: "F1", severity: "P1", file: "src/lib.rs", message: "fix me" }],
  summary: "needs work",
  ...overrides,
});

const check = (overrides = {}) => ({
  id: "c1",
  command: "cargo",
  args: ["test", "--release", "-p", "xai-grok-pager", "acp::live_feed"],
  timeoutMs: 600_000,
  ...overrides,
});

const candidate = (overrides = {}) => ({
  revision: REV,
  summary: "synthetic candidate",
  checks: [check()],
  smoke: ["cargo test --release -p xai-grok-pager acp::live_feed -- --list"],
  ...overrides,
});

// parsePacket -------------------------------------------------------------

test("parsePacket accepts a valid packet with exact shape", () => {
  assert.deepEqual(parsePacket(VALID_PACKET), VALID_PACKET);
});

test("parsePacket accepts each allowed id and rejects others", () => {
  for (const id of ["f3", "cass", "claude-settings"])
    assert.equal(parsePacket(packet({ id })).id, id);
  for (const id of ["f2", "", "claude", null, 3]) assert.throws(() => parsePacket(packet({ id })));
});

test("parsePacket rejects unknown/missing keys", () => {
  assert.throws(() => parsePacket(packet({ extra: 1 })), /unknown keys: extra/);
  assert.throws(() => parsePacket(packet({ context: null })), /missing keys: context/);
});

test("parsePacket rejects nonempty violations", () => {
  assert.throws(() => parsePacket(packet({ goal: "   " })));
  assert.throws(() => parsePacket(packet({ files: [] })));
  assert.throws(() => parsePacket(packet({ constraints: [] })));
  assert.throws(() => parsePacket(packet({ acceptance: [] })));
  assert.throws(() => parsePacket(packet({ context: [""] })));
  assert.throws(() => parsePacket(packet({ startingRef: "" })));
});

test("parsePacket rejects wildcard, absolute, and traversal file paths", () => {
  for (const bad of [
    "crates/*/src.rs",
    "src/**",
    "a?.rs",
    "/abs/path.rs",
    "C:/x.rs",
    "../escape.rs",
    "a/../b.rs",
    "a\\\\b.rs",
    "dir//x.rs",
  ]) {
    assert.throws(
      () => parsePacket(packet({ files: [bad] })),
      undefined,
      `expected rejection of ${bad}`,
    );
  }
});

test("parsePacket validates builder and rejects wrong thinking", () => {
  assert.throws(
    () => parsePacket(packet({ builder: { provider: "xai", model: "grok-4", thinking: "low" } })),
    /thinking/,
  );
  assert.throws(() =>
    parsePacket(packet({ builder: { provider: "xai", model: "", thinking: "high" } })),
  );
  assert.throws(() => parsePacket(packet({ builder: null })));
});

test("parsePacket rejects non-object and array input", () => {
  assert.throws(() => parsePacket(null));
  assert.throws(() => parsePacket("x"));
  assert.throws(() => parsePacket([VALID_PACKET]));
});

// parsePackets ------------------------------------------------------------

test("parsePackets accepts exactly three packets with the three distinct ids", () => {
  const packets = [packet({ id: "f3" }), packet({ id: "cass" }), packet({ id: "claude-settings" })];
  assert.deepEqual(parsePackets(packets), packets);
  assert.deepEqual(
    parsePackets(packets.slice().reverse()).map((p) => p.id),
    ["claude-settings", "cass", "f3"],
  );
});

test("parsePackets rejects wrong count and duplicate ids", () => {
  assert.throws(() => parsePackets([]), /exactly 3/);
  assert.throws(() => parsePackets([packet({ id: "f3" }), packet({ id: "cass" })]));
  assert.throws(
    () => parsePackets([packet({ id: "f3" }), packet({ id: "f3" }), packet({ id: "cass" })]),
    /distinct/,
  );
  assert.throws(() => parsePackets(null));
});

// digest ------------------------------------------------------------------

test("digest is key-order independent and recursively sorted", () => {
  const a = { b: 1, a: { y: [{ d: 2, c: 1 }], x: 1 } };
  const b = { a: { x: 1, y: [{ c: 1, d: 2 }] }, b: 1 };
  assert.equal(digest(a), digest(b));
  assert.notEqual(digest({ a: 1 }), digest({ a: 2 }));
  assert.notEqual(digest({ a: { b: 1 } }), digest({ a: [1] }));
});

test("digest matches canonical sorted JSON sha256 exactly", () => {
  assert.equal(digest({ a: "x", b: ["q", "r"] }), digest({ b: ["q", "r"], a: "x" }));
  // manual reference: canonical form of {a:1,b:{c:2,d:1}} is '{"a":1,"b":{"c":2,"d":1}}'
  const expected = createHash("sha256").update('{"a":1,"b":{"c":2,"d":1}}').digest("hex");
  assert.equal(digest({ b: { d: 1, c: 2 }, a: 1 }), expected);
  assert.equal(digest("plain"), digest("plain"));
});

// parseReview -------------------------------------------------------------

test("parseReview accepts ready/changes/blocked with valid findings", () => {
  assert.equal(parseReview(review({ verdict: "ready", findings: [] })).verdict, "ready");
  assert.equal(parseReview(review({ verdict: "changes" })).verdict, "changes");
  assert.equal(parseReview(review({ verdict: "blocked", findings: [] })).verdict, "blocked");
  assert.equal(parseReview(review({ verdict: "blocked" })).findings.length, 1);
});

test("parseReview enforces ready=0 findings and changes>=1", () => {
  assert.throws(() => parseReview(review({ verdict: "ready" })), /ready.*zero findings/);
  assert.throws(
    () => parseReview(review({ verdict: "changes", findings: [] })),
    /changes.*at least one/,
  );
});

test("parseReview validates 40-hex revision and 64-hex goalDigest", () => {
  assert.throws(() => parseReview(review({ revision: "a".repeat(39) })));
  assert.throws(() => parseReview(review({ revision: "A".repeat(40) })));
  assert.throws(() => parseReview(review({ revision: REV.slice(0, 39) + "g" })));
  assert.throws(() => parseReview(review({ goalDigest: "c".repeat(63) })));
});

test("parseReview validates severity and finding shape", () => {
  for (const sev of ["P0", "P1", "P2"])
    assert.equal(
      parseReview(review({ findings: [{ id: "x", severity: sev, file: "f", message: "m" }] }))
        .findings[0].severity,
      sev,
    );
  assert.throws(() =>
    parseReview(review({ findings: [{ id: "x", severity: "P3", file: "f", message: "m" }] })),
  );
  assert.throws(() =>
    parseReview(review({ findings: [{ id: "x", severity: "P1", file: "f", extra: 1 }] })),
  );
  assert.throws(() => parseReview(review({ findings: "nope" })));
  assert.throws(() => parseReview(review({ summary: "  " })));
  assert.throws(() => parseReview(review({ unknown: 1 })));
});

// parseCandidate ----------------------------------------------------------

test("parseCandidate accepts a valid cargo-test candidate", () => {
  assert.deepEqual(parseCandidate(candidate()), candidate());
});

test("parseCandidate accepts all approved crates", () => {
  for (const crate of APPROVED_CRATES) {
    const c = candidate({
      checks: [check({ args: ["test", "--release", "-p", crate, "--", "filter::x"] })],
    });
    assert.equal(parseCandidate(c).checks[0].args[3], crate);
  }
});

test("parseCandidate requires cargo test --release scoped to an approved crate", () => {
  assert.throws(
    () => parseCandidate(candidate({ checks: [check({ command: "rustc" })] })),
    /command/,
  );
  assert.throws(
    () =>
      parseCandidate(
        candidate({ checks: [check({ args: ["build", "--release", "-p", "xai-grok-pager"] })] }),
      ),
    /cargo test/,
  );
  assert.throws(
    () =>
      parseCandidate(
        candidate({ checks: [check({ args: ["test", "-p", "xai-grok-pager", "f"] })] }),
      ),
    /--release/,
  );
  assert.throws(
    () =>
      parseCandidate(
        candidate({
          checks: [check({ args: ["test", "--release", "-p", "some-other-crate", "f"] })],
        }),
      ),
    /approved crate/,
  );
  assert.throws(
    () => parseCandidate(candidate({ checks: [check({ args: ["test", "--release", "f"] })] })),
    /approved crate/,
  );
  assert.throws(
    () => parseCandidate(candidate({ checks: [check({ args: ["test", "--release", "-p"] })] })),
    /approved crate/,
  );
});

test("parseCandidate requires a nonempty scoped filter (post '--' or positional)", () => {
  assert.throws(
    () =>
      parseCandidate(
        candidate({ checks: [check({ args: ["test", "--release", "-p", "xai-grok-pager"] })] }),
      ),
    /scoped/,
  );
  assert.throws(
    () =>
      parseCandidate(
        candidate({
          checks: [check({ args: ["test", "--release", "-p", "xai-grok-pager", "--"] })],
        }),
      ),
    /scoped/,
  );
  // accepted: positional filter and post-'--' filter; -p value is not a filter
  assert.equal(
    parseCandidate(
      candidate({
        checks: [check({ args: ["test", "--release", "-p", "xai-grok-pager", "lib::tests"] })],
      }),
    ).checks.length,
    1,
  );
  assert.equal(
    parseCandidate(
      candidate({
        checks: [
          check({ args: ["test", "--release", "-p", "xai-grok-pager", "--", "lib::tests"] }),
        ],
      }),
    ).checks.length,
    1,
  );
  assert.equal(
    parseCandidate(
      candidate({
        checks: [check({ args: ["test", "--release", "--lib", "-p", "xai-grok-traces", "unit"] })],
      }),
    ).checks.length,
    1,
  );
});

test("parseCandidate rejects banned harness/build tokens", () => {
  for (const token of ["--ignored", "--no-run", "--list", "--doc", "--workspace", "--target-dir"]) {
    const args = ["test", "--release", "-p", "xai-grok-pager", "--", "f", token];
    assert.throws(
      () => parseCandidate(candidate({ checks: [check({ args })] })),
      new RegExp(token.replace(/^-+/, "")),
      `expected rejection of ${token}`,
    );
  }
});

test("parseCandidate rejects release flag after harness separator", () => {
  assert.throws(
    () =>
      parseCandidate(
        candidate({
          checks: [check({ args: ["test", "-p", "xai-grok-pager", "--", "goal", "--release"] })],
        }),
      ),
    /precede harness/,
  );
});

test("parseCandidate rejects package expansion, alternate target dirs and flag-only filters", () => {
  for (const extra of [
    ["--", "--nocapture"],
    ["--", "--test-threads", "1"],
    ["--", "--skip", "unrelated"],
    ["filter", "--target-dir=/tmp/fresh"],
    ["filter", "-p", "unapproved"],
    ["filter", "--package=unapproved"],
    ["filter", "--config=x"],
  ]) {
    assert.throws(() =>
      parseCandidate(
        candidate({
          checks: [check({ args: ["test", "--release", "-p", "xai-grok-pager", ...extra] })],
        }),
      ),
    );
  }
});

test("parseCandidate rejects shell-like characters in args", () => {
  for (const bad of ["a;b", "a$b", "a`b", "a\nb", "a|b", "a>b", "a&b", "a(b"]) {
    assert.throws(
      () =>
        parseCandidate(
          candidate({
            checks: [check({ args: ["test", "--release", "-p", "xai-grok-pager", "--", bad] })],
          }),
        ),
      undefined,
      `expected rejection of ${JSON.stringify(bad)}`,
    );
  }
  assert.throws(() =>
    parseCandidate(
      candidate({
        checks: [check({ args: ["test", "--release", "-p", "xai-grok-pager", "--", ""] })],
      }),
    ),
  );
});

test("parseCandidate enforces check count and timeout bounds", () => {
  assert.throws(() => parseCandidate(candidate({ checks: [] })), /nonempty/);
  const tooMany = Array.from({ length: MAX_CHECKS + 1 }, (_, i) => check({ id: `c${i}` }));
  assert.throws(
    () => parseCandidate(candidate({ checks: tooMany })),
    new RegExp(`at most ${MAX_CHECKS}`),
  );
  const okMany = Array.from({ length: MAX_CHECKS }, (_, i) =>
    check({ id: `c${i}`, args: ["test", "--release", "-p", "xai-grok-pager", `f${i}`] }),
  );
  assert.equal(parseCandidate(candidate({ checks: okMany })).checks.length, MAX_CHECKS);
  assert.throws(() => parseCandidate(candidate({ checks: [check({ timeoutMs: 0 })] })), /positive/);
  assert.throws(
    () => parseCandidate(candidate({ checks: [check({ timeoutMs: 1.5 })] })),
    /positive/,
  );
  assert.throws(
    () => parseCandidate(candidate({ checks: [check({ timeoutMs: MAX_TIMEOUT_MS + 1 })] })),
    /<=/,
  );
  assert.equal(
    parseCandidate(candidate({ checks: [check({ timeoutMs: MAX_TIMEOUT_MS })] })).checks[0]
      .timeoutMs,
    MAX_TIMEOUT_MS,
  );
});

test("parseCandidate rejects duplicate check ids and validates smoke strings", () => {
  assert.throws(() => parseCandidate(candidate({ checks: [check(), check()] })), /distinct/);
  assert.throws(() => parseCandidate(candidate({ smoke: [""] })));
  assert.throws(() => parseCandidate(candidate({ smoke: "not-array" })));
  assert.equal(parseCandidate(candidate({ smoke: [] })).smoke.length, 0);
  assert.throws(() => parseCandidate(candidate({ unknown: true })));
});

// parseReviewBound --------------------------------------------------------

test("parseReviewBound accepts 1..5 and rejects others", () => {
  for (const v of [1, 2, 3, 4, 5]) assert.equal(parseReviewBound(v), v);
  for (const v of [0, -1, 6, 2.5, "3", null]) assert.throws(() => parseReviewBound(v));
});

// reviewRoute -------------------------------------------------------------

test("reviewRoute returns ready for verdict ready regardless of round", () => {
  const r = review({ verdict: "ready", findings: [] });
  assert.equal(reviewRoute(r, REV, GD, 1, 3), "ready");
  assert.equal(reviewRoute(r, REV, GD, 3, 3), "ready");
});

test("reviewRoute returns repair for non-ready below the round limit", () => {
  assert.equal(reviewRoute(review(), REV, GD, 1, 3), "repair");
  assert.equal(reviewRoute(review({ verdict: "blocked" }), REV, GD, 2, 3), "blocked");
});

test("reviewRoute returns blocked for non-ready at the limit", () => {
  assert.equal(reviewRoute(review(), REV, GD, 3, 3), "blocked");
  assert.equal(reviewRoute(review({ verdict: "blocked" }), REV, GD, 2, 2), "blocked");
});

test("reviewRoute throws on revision and goalDigest mismatch", () => {
  assert.throws(() => reviewRoute(review(), REV2, GD, 1, 3), /revision/);
  assert.throws(() => reviewRoute(review(), REV, GD2, 1, 3), /goalDigest/);
  assert.throws(() => reviewRoute(review(), "zzz", GD, 1, 3));
});

test("reviewRoute throws on invalid or out-of-range rounds", () => {
  assert.throws(() => reviewRoute(review(), REV, GD, 0, 3));
  assert.throws(() => reviewRoute(review(), REV, GD, 4, 3));
  assert.throws(() => reviewRoute(review(), REV, GD, 1, 0));
  assert.throws(() => reviewRoute(review(), REV, GD, 1.5, 3));
  // non-ready exactly at limit is blocked, not thrown
  assert.equal(reviewRoute(review(), REV, GD, 3, 3), "blocked");
});

// parseModel --------------------------------------------------------------

test("parseModel accepts provider/model with thinking high", () => {
  assert.deepEqual(parseModel({ provider: "xai", model: "grok-4", thinking: "high" }), {
    provider: "xai",
    model: "grok-4",
    thinking: "high",
  });
});

test("parseModel rejects empty, unknown keys, and non-high thinking", () => {
  assert.throws(() => parseModel({ provider: "", model: "grok-4", thinking: "high" }));
  assert.throws(() => parseModel({ provider: "xai", model: "grok-4", thinking: "medium" }));
  assert.throws(() => parseModel({ provider: "xai", model: "grok-4", thinking: "high", extra: 1 }));
  assert.throws(() => parseModel({ provider: "xai" }));
});
