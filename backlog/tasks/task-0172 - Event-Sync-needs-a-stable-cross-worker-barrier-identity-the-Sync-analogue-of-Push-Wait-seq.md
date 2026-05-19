---
id: TASK-0172
title: >-
  Event::Sync needs a stable cross-worker barrier identity (the Sync analogue of
  Push/Wait seq)
status: To Do
assignee: []
created_date: '2026-05-19 00:08'
labels:
  - M2
  - backend
  - contract
dependencies:
  - TASK-0124
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered by TASK-0124. Event::Push/Wait carry a stable cross-worker SeqTag; Event::Sync carries only participants+kind, NO stable cross-worker identity. The pre-TASK-0124 multi_worker recovered barrier identity from a GLOBAL acfg tree walk (walk_assign_sync_ids); the EventList-only path cannot recover that from disjoint per-worker lists in general. TASK-0124's multi_worker assigns barrier id by per-worker PRE-ORDER Sync index, which is byte-identical to the old ids ONLY for UNIFORM barriers (every Sync has the same participant set — true for 02-split's three {host,w0} barriers). It VALIDATES uniformity and fails loud (EmitError::ContractGap) on a partial/non-uniform-barrier schedule rather than emit a wrong barrier graph. The robust fix is to give Event::Sync a stable id (the Sync analogue of TASK-0156 FireBinding / TASK-0159 Event::Loop / Push-Wait seq) so partial-barrier multi-worker schedules can be lowered correctly. Until then, partial-barrier multi-worker is a typed codegen error, not a wrong binary.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Event::Sync carries a stable cross-worker barrier identity (or an equivalent join key) so disjoint per-worker EventLists agree on barrier identity without a global ACFG walk
- [ ] #2 pthreads-sync multi_worker uses that identity instead of the per-worker pre-order-index heuristic
- [ ] #3 a partial/non-uniform-barrier multi-worker schedule lowers correctly (no ContractGap rejection)
- [ ] #4 e2e 02-split + determinism stay byte-identical
<!-- AC:END -->
