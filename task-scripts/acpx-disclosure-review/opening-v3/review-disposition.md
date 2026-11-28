# Opening v3: answered native gates

Fable's review is in `../opening-v2/fable-5.2-review.md`. The blocking answered-wait clause is corrected in this render. Product excerpts are from `b9017d23`; the attached design is the version published with this packet.

The ledger already distinguishes native ACPX waits answered by a continuation. Preserve the existing classification, keep those runs individually visible as answered under Recent outcomes, and exclude them from waiting rows, groups, and counts. The acceptance fixture uses five unanswered and two answered waits. PIW acceptance and reconciliation stay in packet 5.

This revision addresses the answered-wait blocker only. Fable's other observations remain on record; they were not promoted to dispatch blockers or folded into this correction. Dry render only. No worker or build was started.
