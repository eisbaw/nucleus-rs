---
id: TASK-0235
title: >-
  Test gaps: 0-barrier multi-worker + asymmetric-participant barrier for
  pthreads-async Plan
status: To Do
assignee: []
created_date: '2026-05-22 00:14'
labels:
  - M4
  - backend
  - test-coverage
dependencies:
  - TASK-0234
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-21 review-gate C.2 / F.3 finding (commit 7bacc17): the two new TASK-0234 tests pin barrier_participants correctness for the COMMON case (non-empty + uniform participants per tag) but miss two edge scenarios:

(a) 0-barrier multi-worker schedule. The Plan's barrier_participants should be EMPTY when no Event::Sync exists. A future walker regression that synthesised a phantom tag would pass the existing tests. No e2e fixture has zero barriers (every multi-worker schedule produces at least one inject_syncs barrier), so this needs a SYNTHETIC per_worker test fixture (mirror skeleton.rs's two_workers_empty helper but with one Event::Push + one Event::Wait + zero Event::Sync).

(b) Asymmetric-participant barriers. pthreads-sync's partial_nonuniform_barrier_multi_worker_lowers_correctly fixture covers this for the sync backend. An analogous test for pthreads-async's Plan would assert: barrier_participants[tag1].len() != barrier_participants[tag2].len() when the schedule has non-uniform barriers. Confirms the docstring claim 'partial / non-uniform barriers lower correctly'.

These are deferrable: Wave B-2's emit-string tests will naturally exercise (a) (empty-map iteration), and pthreads-sync's existing fixture demonstrates (b) works at the sync level. Filing as a separate follow-up to keep TASK-0234 focused and avoid scope creep.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Test fixture (a): synthetic 2-worker per_worker with Push/Wait but no Sync; asserts barrier_participants is empty.
- [ ] #2 Test fixture (b): asymmetric-participant barriers (likely synthetic, mirror pthreads-sync's partial_nonuniform fixture); asserts barrier_participants entries have differing parts.len() across tags.
<!-- AC:END -->
