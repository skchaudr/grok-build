import assert from "node:assert/strict";
import test from "node:test";
import { verifyApprovedBundle } from "../lib/approved-packets.mjs";
import { digest } from "../lib/landing-contracts.mjs";
test("approved bundle must match server terminal receipt and verified human provenance", async () => {
  const bundle = { runId: "fixture", approval: { choice: "approve" }, packets: ["fixture"] };
  let state = {
    status: "completed",
    workflowName: "prepare-landing",
    finalOutput: { status: "approved", digest: digest(bundle) },
    humanDecision: { nodeId: "packet_gate", provenance: "human", response: { choice: "approve" } },
  };
  const client = {
    async getRun() {
      return { state };
    },
    async hydrateContent(_, v) {
      return v;
    },
  };
  assert.equal(await verifyApprovedBundle(bundle, client), bundle);
  await assert.rejects(
    verifyApprovedBundle({ ...bundle, packets: ["tampered"] }, client),
    /exact output/,
  );
  state = { ...state, humanDecision: { provenance: "timeout", response: { choice: "approve" } } };
  await assert.rejects(verifyApprovedBundle(bundle, client), /human-approved/);
  await assert.rejects(
    verifyApprovedBundle(bundle, {
      ...client,
      async getRun() {
        return null;
      },
    }),
    /unavailable/,
  );
});
