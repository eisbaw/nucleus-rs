---
id: TASK-0407
title: 'Hardening: dead-code / dead-reexport / limitation audit (review pass)'
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 07:35'
updated_date: '2026-06-01 08:15'
labels:
  - hardening
  - dead-code-audit
  - review-pass
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 endgame. The TEST-COVERAGE hardening wave is exhausted (prove-the-check-bites across all typed error enums SATURATED per TASK-0400/0401/0402/0404; serde round-trips + determinism + parser ParseError SATURATED per TASK-0406; doc-citation fences saturated; parser fuzz TASK-0399). The remaining named hardening dimension is REVIEW-PASS type: a dead-code / dead-reexport / limitation audit.

SCOPE: (1) dead pub re-exports (memory feedback-visibility-tighten-doclink-trap: backend-common pub mod re-exports are often DEAD -- remove, do not narrow; narrowing doc-linked modules breaks intra-doc-links SILENTLY, so run cargo doc on any visibility change). (2) #[allow(dead_code)] sites -- are they still load-bearing or removable? (3) structurally-dead error variants already FOUND (UnsupportedPartitionKind, InnerRepeatNotFound, BlockTransformError::NotDivisible) -- confirm each is either documented-unreachable or removable; no NEW ones. (4) cargo +nightly udeps / cargo machete for unused deps if available in the dev shell.

METHOD (load-bearing, forward-carried): coverage/inventory audits UNDER-count -- re-derive denominators structurally; grep BOTH tests/ AND inline cfg(test) mods; adversarially try to FALSIFY any saturation/dead claim, do not self-certify (memory feedback-coverage-audit-undercount-recurring; 3 firings cycle-236). Deliverable = findings -> precise follow-up tasks (and/or small dead-code removals through the normal gate+review). LOWER leverage than the test-coverage wave; best in a FRESH context.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## forward-carried from TASK-0408 (cycle-236 doc-lie sweep)

Two lessons that bear on the dead-code/limitation audit:

1. eval_const-attribution conflation (recurring why-claim lie): TASK-0408 found a doc-lie where THREE sibling docstrings (partition_workers / partition_rows / common.rs map_band_error + the InvalidRange variant doc) attributed an inverted-range hi-less-than-lo guard to "the link step's eval_const invariant" -- which has NO such invariant. When the 0407 audit touches a documented-unreachable variant (e.g. UnsupportedPartitionKind, InnerRepeatNotFound, BlockTransformError::NotDivisible), VERIFY the documented reason for unreachability by tracing the actual gate, not by trusting the doc's named gate. A variant doc that says "X cannot reach this because gate Y rejects it" is a CLAIM -- grep that Y actually rejects it. Fixed in commit 3809fdf.

2. PartitionBandError::InvalidRange is reachable-but-defensive, NOT structurally dead: I confirmed PartitionBandError::InvalidRange IS reachable (compute_partition_bands returns it on hi-less-than-lo) and now has a test pinning the partition-pass backstop mapping. So if 0407 considers any PartitionBandError variant for the structurally-dead list: InvalidRange is NOT dead (defensive-reachable + now tested); InsufficientWork is live; ZeroWorkers is pre-empted by the worker-count check (len-less-than-2) so it is the band-helper-level defensive-only variant -- documented-unreachable from the partition passes but reachable via the n_workers==0 helper precondition. Classify ZeroWorkers as documented-defensive, not removable (the helper is shared, a future caller could pass 0).
<!-- SECTION:NOTES:END -->
