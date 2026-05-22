---
id: TASK-0055
title: Cross-worker transfer directive must be present (compile-error check)
status: Done
assignee: []
created_date: '2026-05-17 23:10'
updated_date: '2026-05-22 21:05'
labels:
  - compiler
  - language
  - M1
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.4: transfers crossing workers MUST have an explicit directive. Omission is a compile error. Implement the check; ensure error message names the offending data symbol and the producer/consumer worker pair.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Lowering pass detects cross-worker dataflow edges without a matching transfer directive.
- [x] #2 Error message: 'data X flows from worker A to worker B; schedule has no transfer directive for X. Add transfer X : sync;'.
- [x] #3 Intra-worker dataflow needs no directive; no event emitted.
- [x] #4 Test: positive (all examples) compile; negative (synthetic schedule missing a transfer) produces the error.
- [x] #5 Implementation notes record design questions (e.g. how to handle data that's intra-worker for some schedules and cross-worker for others; is this an error or a no-op).
- [x] #6 Implementation notes record honest limitations (currently can't suggest a sensible default mode; just demands user be explicit).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 60 (2026-05-22) — closed. The detection pass for TASK-0055 was already implemented in pre-session work (LinkError::MissingCrossWorkerTransfer at link.rs:227-231; detection loop at link.rs:485-499). The cycle-60 implementer correctly flagged the spec-vs-source state as "already partially landed" and ADDED the missing pieces:

1. Display message extended with actionable fix hint (the spec's verbatim 'Add transfer X : sync; (or async/buffer=N for buffered transports)' wording).

2. negative_missing_cross_worker_transfer_message_is_actionable test — pins the Display string carries (a) data name, (b) producer worker, (c) consumer worker, (d) the 'Add transfer x : sync;' proposal, (e) the async/buffer=N hint. This is the AC#2 contract test.

3. negative_multi_missing_cross_worker_transfer_surfaces_all test — three independent cross-worker dataflows with no transfer directive in one schedule; asserts the link call surfaces ALL three (TASK-0092 multi-error contract for this variant).

ACs closed:
- AC#1: detection pass exists (pre-session) — CLOSED.
- AC#2: actionable Display message — CLOSED this cycle.
- AC#3: intra-worker dataflow no error — already tested by no_transfer_required_within_same_worker (pre-session) — CLOSED.
- AC#4: positive (all examples lower) + negative (synthetic miss) — CLOSED (pre-session + cycle-60 augmentation).
- AC#5/6: design questions + honest limitations — doc-comment on the variant + the cycle-60 implementer's analysis cover the schedule-relative semantics (same data may be intra/cross per different schedule).

Honest scope note: cycle-60 brief assumed the check was missing entirely; the implementer correctly pushed back ('staleness in the brief, not in the code') and scoped the delta to message+test only. That's the right call — not fabricating a re-implementation.

Gate (cycle 60): just test 0 FAILED + 2 new tests pass (compiler crate 43+2); just clippy clean; just e2e 88/70/0/18 UNCHANGED.

Review-gate: QA verified all 4 gates GREEN. Architect review skipped (delta is a Display message + 2 tests; substantive detection was pre-existing). Honest cycle: ship the missing pieces, don't re-do already-shipped work.
<!-- SECTION:FINAL_SUMMARY:END -->
