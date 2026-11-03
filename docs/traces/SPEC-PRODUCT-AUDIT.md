# Product spec audit — 2026-11-03

Read-only audit of implementation; proposed spec corrections for Sab to review.
Code baseline: `11063c7d`. Reviewed draft SHA-256:
`ea802c3e7e37a445a3ed0279f3376c13065f99695daec0f7d0faa447379826eb`.
The draft is being edited concurrently; findings refer to this snapshot.

## Governing requirement

Sab: stay as close to grok-build as possible; add only when necessary or when
there is significant value. Standalone launch is an entry point requirement.
It does not authorize replacing another product or removing its capabilities.

The proposed direction fits: harness readers feed Grok's session event handlers;
Grok owns rendering, grouping, navigation, plans and child views. Existing traces
discovery, receipts, matching, remote access and dpi reads remain useful.

## Findings and proposed corrections

### 1. Immediate tool rows became a status-line-only promise

Draft lines 134–138 delay the row until its transcript arrives. The first draft
in this review promised a running row as soon as the receipt or span arrived.
Claude's Rex pane explicitly described that substitution as not yet approved.
This changes what Sab can inspect while a tool runs.

Proposed wording: a receipt opens the native running tool row immediately;
transcript content fills that same row by call ID. If native ordering or update
handling prevents this, show the specific failure and proposed adjustment to Sab.
Do not silently trade the row for the status line.

The tracker removes completed calls from `pending_tools`; later updates become
orphans (`acp/tracker.rs`, `handle_tool_call_update`). The merge must account for
that. A call's reported completion and the availability of its output are separate
facts: missing output must not silently turn an ended or failed call into a
still-running call. The exact native presentation needs a demonstrated solution.

### 2. Existing entry point is not an already-connected session viewer

Draft line 32 overstates `app/dispatch/standalone_traces.rs`. It creates an
`AgentView` and opens the old traces modal; it does not load external content into
Grok's session view. `attached_as_viewer` tracks turn ownership; root composer
hiding needs an explicit extension (`app/agent_view/role.rs`).

Plans require the app handler; the tracker alone ignores them. Child spawn
handling creates child views (`app/acp_handler/session_notification.rs`). These
are good reuse targets. A converter-to-tracker shortcut would omit behavior.

Proposed work boundary: adapt event input, add a read-only root mode, connect
external child replay, extend nested routing and supply recorded durations.
Reuse existing draw functions and action handling. Any additional view, key map
or state model needs a concrete missing capability and explanation of its value.

### 3. Native key behavior needs to survive inside children too

Draft line 120 says Esc goes back because cancellation is unnecessary. Grok still
needs Esc for search and inner views. Its takeover already closes only from bare
scrollback (`app/agent_view/subagent_takeover.rs`, `intercept_takeover_input`).
The child policy also currently hides the command palette and denies actions
including the Tasks pane (`app/agent_view/child_action_filter.rs`). Therefore
promising all native controls everywhere requires explicit routing work.

Proposed wording: preserve Grok's inner-view dismissal order; Esc returns to the
parent or list from bare scrollback. Check which native controls apply at root
and child depth. Extend those paths only for required capabilities.

### 4. Phoenix and wake claims need a working example

Keep Phoenix's specified role. Its integration is unfinished, with concrete
differences between the draft and the installed Pi emitter:

- `~/.pi/agent/extensions/otlp-observe.ts` already retains tool start time and
  exports a tool span at result time. Its session and turn spans use immediate
  start/end. The blanket statement "today start equals end" is inaccurate.
- Its default Python path is under `~/phoenix-lab`, absent on this Mini. The
  shipper accepts only selected attributes, so adding identifiers in TypeScript
  alone would still drop them. Its session ID source also needs comparison with
  the receipt/transcript session ID before using it for matching.
- `~/.local/share/agent-obs/bin/otlp-ship.py` ends every span before flushing.
  These files do not establish how Phoenix exposes a currently running remote
  call or an open session root. Prove that path before depending on it for live
  status. This is an implementation question, not grounds to omit Phoenix.
- `wake.rs` is a local notification socket. The inspected traces source calls
  `wake::notify()` from hook ingestion; the modal still polls. Transcript-only
  appends and new remote spans have no demonstrated route to that local socket.

Proposed acceptance: a long-running local call and remote call appear before
completion, retain their IDs through completion, and fill with transcript output.
Also demonstrate a transcript-only append and Phoenix going offline. Name the
actual wake source or bounded polling fallback; do not promise universal push
delivery from the existence of `wake.rs`.

### 5. Replacement and supersession need explicit boundaries

Draft line 162 broadly removes drawing code, keys, footers and associated tests.
The earlier MVP explicitly preserves `follow --json` shapes for trace-deck.
The product draft does not restate that compatibility requirement.

Proposed wording: preserve standalone entry points, CLI/JSON contracts and
trace-deck integration unless Sab explicitly changes them. Replace only the
pager presentation made redundant by the connected Grok session view. List any
proposed file deletions and obtain the special deletion permission required by
AGENTS.md. Retain behavioral coverage when replacing its old test harness.

Superseding old specs should retain a short requirement-to-destination list.
Keep useful requirements, identify those satisfied by native Grok behavior, and
record Sab's decisions for intentional changes. Earlier custom UI proposals
should not be implemented merely because an older document mentioned them.

The harness scope changed during this review. This audit has not established
whether Sab approved each scope change; it does not declare those changes wrong.
Record that decision instead of inferring it from a rewritten draft.

### 6. Future control requires more than revealing a composer

Retaining harness/session/pane identity is reasonable. Claude's pane described
Phase 2 as showing the composer and routing input, but harness control paths,
child addressing and lifecycle semantics remain work to specify and prove.
The current draft appropriately defers that specification. Do not claim Phase 1
has delivered control, or add control abstractions before a real integration
requires them. Native live Grok attachment must also obey the read-only mode;
"there is no agent to send to" is not sufficient for that path.

## Review checklist for each implementation slice

1. Name the user-visible requirement and existing Grok/traces path being reused.
2. Identify added behavior and why the existing path cannot provide it.
3. Compare the changed files with that scope, including removed behavior.
4. Verify the relevant real-pane behavior against native Grok at full and half
   width. For the full product: past replay, immediate running rows, full output,
   errors/durations, child/grandchild/back/reopen, and offline operation.
5. Review targeted tests for those behaviors and existing consumer compatibility.
   Reviewer agreement alone is not acceptance evidence. Check Mini and Air
   installation against the reviewed commit when deployment is authorized.

This is a review checklist, not a new runtime subsystem or a requirement that
Sab approve every routine code edit. Product changes and deletions remain his
decisions. No product code, system settings or other agent's draft was changed
by this audit; no implementation tests were run.
