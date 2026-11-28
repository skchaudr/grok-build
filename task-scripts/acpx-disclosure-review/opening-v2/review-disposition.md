# Opening-screen review disposition

Grok 4.7 published the original review at `75d16429` in `../opening-v1/grok-4.7-review.md`. Sonnet landed the run2 ledger at `1f7fe9b1` before this revision. This revision uses product source at `4919ef57`.

1. **Accepted and fixed: duplicate ledger scope.** `acpx-opening-ledger-delta` preserves the landed finished grouping, rail/count/path, and list copy. It asks for waiting groups, section classification, and the ledger half of the compact inline preview. The old full-ledger render is historical.
2. **Accepted and held: preview depends on landed ledger APIs.** The revised preview definition explicitly waits for the ledger delta. The next render must attach the landed inline line type and the three ledger methods. There is no new dispatchable preview render in this directory.
3. **Accepted in substance and fixed: continuation context must be post-run2.** The ledger delta includes `build_flow_rows`, `append_continuations`, `continuation_lines_for`, `render_flows`, and the entire `render_flow_row`. The old excerpts already contained the old `render_flow_row`; the missing evidence was packet 2's landed executed-path continuation. The new spec covers the first question fragment and row path as well as the continuation lines so the inline preview does not duplicate them.

Nonblocking notes and unverified items remain in the original review. The revised ledger packet was dry-rendered and checked against the pinned source. No worker or product build was started for this revision.
