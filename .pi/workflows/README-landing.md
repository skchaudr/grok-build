# Small landing workflows

These are project workflows, not workflow-engine extensions. No new primitive,
server changes, hidden poller, or per-agent turn quota is introduced.

## What runs where

1. **prepare-landing** runs in a separate detail-preparation Pi using the same
   model as the coordinating parent. It investigates and presents exactly three
   packets: F3, Cass, Claude settings. A protected **approve / revise / stop**
   decision shows the exact packet contents, model assignments, and review bound.
   Approval saves a digest-bound artifact; it does **not** start builders.
2. **launch-landing** consumes that artifact and opens three independent Rex Pi
   sessions, plus one acceptance-check session. Each feature runs the identical
   `lib/task-run.workflow.ts` composition around **reviewed-task**. Separate
   worktrees isolate parallel writers; none runs Cargo.
3. **reviewed-task** keeps the builder in its origin Pi context. Its included
   **independent-review** asks that builder to launch one fresh-context reviewer
   subagent. Detailed feedback stays in that feature session and an artifact.
   The graph routes changes back to the same builder. The caller explicitly
   chooses 1–5 review rounds; no default or model-turn quota. A blocked review,
   unavailable reviewer, failed build stage, or exhausted round budget returns
   a blocked handoff instead of inventing recovery scope.
4. **acceptance-checks** receives only goals and acceptance outcomes, not builder
   reports. It identifies a few decisive checks and missing tests. It does not
   execute Cargo or write product code.
5. **integrate-landing**, started after those four result files exist, lets the
   integration parent assess incoming work, combine commits, resolve overlaps,
   and add missing acceptance tests. The same independent-review composition
   audits the **overall goal against the combined revision**. A failed final
   audit returns evidence for a separately scoped repair, not an automatic loop.
6. A successful audit runs minimal release tests serially in a visible Rex job
   against the existing warm target. Test/install commands are runtime-owned shell
   actions using an explicit Rex session, unique tickets, and bound receipts—not
   model-written command results. Zero-test selections and missing summaries
   fail. A protected delivery decision then authorizes fast-forward + push of
   main and **one normal scripts/install-fork.sh**. A final agent verifies real
   screens and reports required outcomes it could not observe.

Individual runs appear in the existing workflow TUI. This is intentionally not
one synthetic parallel DAG: Pi Workflow includes are sequential; concurrency
comes from separate, visible origin Pi sessions. The launch run records their
identities and finishes. Integration starts from explicit result files, without
polling or waking the coordinator for every builder turn.

## Start only packet preparation

From this parent Rex shell, start **only packet preparation** with:

```sh
bash .pi/workflows/prepare-landing-pi.sh
```

It requires the parent's `PI_PROVIDER`, `PI_MODEL`, and intended `REX_SESSION`;
it opens a fresh visible Pi and starts the protected packet-review workflow.
Nothing has run merely because these definitions exist.

A ready-to-edit input is `landing-input.example.json` beside this document.
Its assignments and five-round review bound are proposals exposed at the human gate.
Open a fresh Pi in Rex with this parent's **provider/model**, then start:

```text
workflow start prepare-landing with:
{
  "cwd": "/Volumes/Mini-SSD/offload/repos/grok-build",
  "goal": "Finish and land the Claude host batch, F3 usage panel, and Cass CLI/TUI search together, preserving existing Pi/Grok behaviour; one shared validation/build/install cycle.",
  "context": [
    "Main is 7d4d917b at authoring time; re-inspect actual refs and working state.",
    "F3: origin/feat/f3-usage-panel; inspect safety commit, tests and .snap.new evidence.",
    "Cass: origin/traces-cass-tui includes origin/traces-cass-search; inspect latest tips.",
    "Claude: docs/briefs/claude-settings-modal-2026-11-14.md and docs/review/claude-settings-modal-build-2026-11-14.md.",
    "Requested host outcomes include Claude resume/branding/version/changelog, running-state display, Agents naming, voice, single-page picker, and live Claude-specific settings.",
    "No feature implementation, Cargo or installation before packet review."
  ],
  "planner": {"provider":"cliproxy","model":"gpt-6.1-sol","thinking":"high"},
  "builders": {
    "f3": {"provider":"opencode-go","model":"grok-4.7","thinking":"high"},
    "cass": {"provider":"opencode-go","model":"minimax-m3","thinking":"high"},
    "claude-settings": {"provider":"opencode-go","model":"glm-5.3-flash","thinking":"high"}
  },
  "reviewer": {"provider":"deepseek","model":"deepseek-flash","thinking":"high"},
  "evaluator": {"provider":"deepseek","model":"deepseek-flash","thinking":"high"},
  "reviewRounds": 5
}
```

**Those builder assignments and five-round bound are a concrete proposal, not
previously accepted operator choices.** Change them before starting if desired;
the packet gate exposes them again. Do not substitute providers after failure.
The actual planner must match the `planner` field; workflow agent nodes do not
select a model by themselves.

Use the workflow tool's `start` action with that input in the new Pi; end the
turn after starting so the step can arrive. The TUI's workflow picker can select
the same project workflow. The gate has no automatic approval deadline.

After approval, `prepare-landing` returns `file` and `digest`. Start
`launch-landing` with `{approvedFile, approvedDigest}`. The evaluator is fixed in
the approved packet artifact, not chosen after the gate. The server's accepted
preparation-run result and human provenance are checked before any launch.

After feature/evaluation runs settle, start `integrate-landing` with the
`integrationInput` path returned by the launcher **and explicit `rexSession`**,
in the intended integration parent Pi. This is a separate explicit start, not a background watcher. A blocked
feature leaves all accepted files and commits available for a focused repair.

## Control and evidence

- Pause/resume/cancel use normal Pi Workflow controls. Cancellation stays terminal;
  it does not route into repair. Delegated Rex jobs/reviewers may outlive the
  originating turn: inspect their actual state before resuming or dispatching a
  replacement. Workflow pause is not proof of child termination.
- Uncertain worktree creation, Git publication, artifact writes or installation
  use manual effects. Never repeat uncertain side effects blindly.
- `.pi/lane/<run-id>/` already is ignored by this repo. It holds packet artifacts,
  worktrees, handoffs, candidate/check receipts, and logs. Do not delete it while
  any associated workflow is active.
- Tool allowlists constrain callable tool names, not filesystem or shell effects.
  Read-only reviewer/no-Cargo boundaries remain task contracts, not an OS sandbox.
  Smoke verification allows read/bash for Rex interaction and checksum inspection;
  it is not hardened against a malicious same-user agent invoking another command.
- A packet's `startingRef` is the pinned contribution to inspect/integrate. All
  feature worktrees start from the approved common main base; the builder ports
  the named contribution onto that base rather than inheriting stale ancestry.
- Model identity is explicit in launch inputs and checked by worker instructions.
  The workflow cannot programmatically attest a child model through a prompt;
  inspect launch receipts before claiming that model completed the work.
- Clean-tree checks include non-ignored untracked files; only the pre-existing
  unrelated `.worktrees/` container is excluded. New tests must be committed.
- The test runner accepts scoped Cargo test descriptors only; shell interpolation,
  alternative target directories, package expansion, build/install commands, and
  flag-only test selections are rejected. Product tests run only in integration.
- No claim of merge-readiness comes from graph validation alone.

## Authoring checks (no product build or model spend)

```sh
export PI_WORKFLOWS_ROOT="$HOME/repos/pi-workflows"
node --test .pi/workflows/tests/landing-contracts.test.mjs
node --import "$PI_WORKFLOWS_ROOT/node_modules/tsx/dist/loader.mjs" --test \
  .pi/workflows/tests/landing-workflows.test.mjs \
  .pi/workflows/tests/landing-engine.test.mjs \
  .pi/workflows/tests/landing-gate.test.mjs \
  .pi/workflows/tests/landing-approval.test.mjs \
  .pi/workflows/tests/landing-visible.test.mjs \
  .pi/workflows/tests/verify-landing.test.mjs \
  .pi/workflows/tests/prepare-landing-launcher.test.mjs
```

These use the actual package loader/composition validator and engine with a
scripted executor and temporary database. They cover ready, repair, exhausted
review, failed review, terminal cancellation, protected gates, stale evidence,
and empty-test failures. They do **not** prove live provider dispatch, cross-Pi
workflow delivery, or end-to-end product behaviour; the first authorized run is
where those are observed.
