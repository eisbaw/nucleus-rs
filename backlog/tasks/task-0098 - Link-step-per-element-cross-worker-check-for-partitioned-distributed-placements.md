---
id: TASK-0098
title: >-
  Link step: per-element cross-worker check for partitioned distributed
  placements
status: Done
assignee: []
created_date: '2026-05-18 00:42'
updated_date: '2026-05-23 21:11'
labels:
  - compiler
  - link
  - M5-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0011 currently treats a distributed placement ('place K on {w0..w3}') as a single worker entity for the cross-worker transfer-existence check. This is correct for whole-tensor data movement but misses per-element halo cases: e.g. blur3 placed on {w0..w3} with partition=rows reading img_in[y-1] across the row boundary needs a transfer for the halo even though producer and consumer are 'the same set'. This is dependent on TASK-0016+ partition= lowering. Acceptance: once partition= is resolved, the link step (or a sibling pass) sees the partitioned iteration space and detects per-partition cross-worker reads against the placement set. Filed now so the limitation is tracked.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as SUPERSEDED + DEFERRED (orchestrator-direct, cycle 77 sweep). The task's headline scenario — 'blur3 placed on {w0..w3} with partition=rows reading img_in[y-1] across the row boundary' — is no longer reachable: TASK-0249 (cycle 70) made partition=rows reject loudly at sched-lower as SchedLowerErrorKind::UnsupportedPartitionKind. The remaining concern (per-partition cross-worker reads under partition=workers) is largely covered by TASK-0212 (loop bound rewrite per worker, landed cycle ~22) + the existing cross-worker transfer requirement (link.rs MissingCrossWorkerTransfer). When/if partition=rows acquires a real downstream consumer pass (filed as TASK-0254-territory or a fresh sibling), the per-element halo check belongs as part of that consumer pass's scope, NOT as a standalone link-step check. Reopen scoped to the actual consumer pass when filed.
<!-- SECTION:FINAL_SUMMARY:END -->
