import { digest } from "./landing-contracts.mjs";

// Verify against the server's accepted terminal output, not a model-authored
// JSON `approval: approve` claim. Reads only; no state ownership or mutation.
export async function verifyApprovedBundle(bundle, client) {
  if (!bundle?.runId) throw new Error("Missing preparation run identity");
  const view = await client.getRun(bundle.runId);
  if (!view) throw new Error("Preparation run unavailable");
  const state = await client.hydrateContent(bundle.runId, view.state);
  const receipt = state.finalOutput;
  if (
    state.status !== "completed" ||
    state.workflowName !== "prepare-landing" ||
    state.humanDecision?.nodeId !== "packet_gate" ||
    receipt?.status !== "approved" ||
    receipt.digest !== digest(bundle) ||
    state.humanDecision?.provenance !== "human" ||
    state.humanDecision?.response?.choice !== "approve"
  )
    throw new Error("Packet bundle is not the exact output of a human-approved preparation run");
  return bundle;
}
