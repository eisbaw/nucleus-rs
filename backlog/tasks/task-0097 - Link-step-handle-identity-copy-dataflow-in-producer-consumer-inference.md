---
id: TASK-0097
title: 'Link step: handle identity-copy dataflow in producer/consumer inference'
status: Done
assignee: []
created_date: '2026-05-18 00:42'
updated_date: '2026-05-28 05:23'
labels:
  - compiler
  - link
  - M1-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0011's link step derives producer/consumer worker entities from Dataflow { lhs, rhs: Call }. An identity-copy dataflow ('D <-- E', RHS is a bare DataRef, no kernel) is currently NOT recorded as a producer edge — its data move is invisible to the cross-worker transfer existence check. None of the in-tree examples exercise this, but a real example will. Acceptance: identity copies attribute the producer to whoever wrote the source datum and the consumer to wherever the LHS is later read; cross-worker check catches the resulting flow when applicable. May require a 'data-symbol scope' or 'data-symbol last-writer worker' map.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== forward-carried from TASK-0347 (cycle 230) ===

This task's original concern (identity-copy producer/consumer attribution
so the cross-worker transfer existence check sees the data move) is now
RESOLVED at root by TASK-0347 commit bcde6b9:
`link::dataflow::propagate_copy_edges` records an identity copy's
producer/consumer transitively over the kernel-driven maps, so a
cross-worker identity copy raises MissingCrossWorkerTransfer exactly as a
kernel-call edge would. The "data-symbol last-writer-worker map" this
task anticipated is implemented as a fixpoint propagation. Pinned by 4
inline tests in nucleus-compiler/tests/link.rs (identity_copy_*). The
ACFG/codegen half (kernel-less Operation) remains open as TASK-0360.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-until-real-example (orchestrator-direct, cycle 77 sweep). Description: 'An identity-copy dataflow (D <-- E, RHS is a bare DataRef, no kernel) is currently NOT recorded as a producer edge... None of the in-tree examples exercise this, but a real example will.' Today: 0 in-tree examples use identity-copy dataflow; all dataflow statements have Call RHS (kernel firing). Reopen when a real example uses 'D <-- E' identity-copy syntax — at that point file a fresh task scoped to the specific producer-edge attribution needed (data-symbol last-writer-worker map, or a sibling check pass). Same deferred-until-trigger pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
